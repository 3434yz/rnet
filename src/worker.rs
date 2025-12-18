use std::sync::Arc;
use std::thread;
use crossbeam::channel::{Receiver, Sender};
use mio::Waker;
use crate::command::{Request, Response, unpack_session_id};

/// 启动一个 Worker 线程
///
/// # 参数
/// * `id`: Worker 的编号（用于日志）
/// * `task_receiver`: 全局任务接收端（所有 Worker 抢这一个队列）
/// * `engine_registry`: 路由表，通过 engine_id 找到对应的 (发送端, 唤醒器)
/// * `processor`: 真正的业务逻辑函数，输入 Job，输出处理后的字节数据
pub fn start_worker<J, F>(
    id: usize,
    task_receiver: Receiver<Request<J>>,
    engine_registry: Vec<(Sender<Response>, Arc<Waker>)>,
    processor: F,
)
where
    J: Send + 'static, // Job 必须能跨线程发送
    F: Fn(J) -> Vec<u8> + Send + Sync + 'static, // 业务逻辑函数
{
    let registry = engine_registry;
    thread::spawn(move || {
        // 循环等待任务。recv() 是阻塞的，没有任务时线程会挂起，不消耗 CPU。
        while let Ok(req) = task_receiver.recv() {

            // 1. 路由解析：这个任务是谁发来的？
            let (engine_id, _token) = unpack_session_id(req.session_id);

            // 2. 执行业务逻辑 (Slow Path)
            // 这里是真正耗时的地方，比如计算哈希、查库等
            let data = processor(req.job);

            // 3. 构造回信
            let response = Response {
                session_id: req.session_id,
                data,
            };

            // 4. 发回 Engine
            // 从注册表中找到对应 Engine 的信箱和门铃
            if let Some((resp_sender, waker)) = registry.get(engine_id) {
                // Step A: 把信扔进 Engine 的信箱
                if let Err(e) = resp_sender.send(response) {
                    eprintln!("Worker {}: Failed to send response to Engine {}: {}", id, engine_id, e);
                    continue;
                }

                // Step B: 按门铃 (唤醒 epoll)
                // 如果不唤醒，Engine 可能会一直卡在 epoll_wait 里，直到有网络 IO 发生才顺便处理这个消息
                if let Err(e) = waker.wake() {
                    eprintln!("Worker {}: Failed to wake Engine {}: {}", id, engine_id, e);
                }
            } else {
                eprintln!("Worker {}: Received task from unknown Engine ID {}", id, engine_id);
            }
        }

        println!("Worker {} stopped.", id);
    });
}