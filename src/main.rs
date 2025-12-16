use bytes::BytesMut;
use core_affinity;
use mio::net::TcpListener;
use rnet::codec::RawCodec;
use rnet::engine::Engine;
use rnet::handler::{Action, Context, EventHandler};
use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::SocketAddr;
use std::thread;

// 业务逻辑：Echo Handler
// Derive Clone/Copy 是因为 EchoHandler 是无状态的 (ZST)，
// 每个 Worker 线程拿一个副本即可。
#[derive(Clone, Copy)]
struct EchoHandler;

impl EventHandler for EchoHandler {
    type Message = BytesMut;
    fn on_open(&self, _ctx: &mut Context) -> Action {
        println!("New Connect");
        Action::None
    }

    fn on_traffic(&self, ctx: &mut Context, in_buf: Self::Message) -> Action {
        ctx.out_buf.extend_from_slice(&in_buf);
        Action::None
    }

    fn on_close(&self, _ctx: &mut Context) -> Action {
        println!("Close Connect");
        Action::None
    }
}

// 保持高性能的 Listener 创建逻辑
fn create_reuse_port_listener(addr_str: &str) -> io::Result<TcpListener> {
    let addr: SocketAddr = addr_str.parse().unwrap();
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;

    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;

    // 保持 4MB Buffer 优化
    let _ = socket.set_recv_buffer_size(4 * 1024 * 1024);
    let _ = socket.set_send_buffer_size(4 * 1024 * 1024);

    socket.bind(&addr.into())?;
    socket.listen(10240)?;

    Ok(TcpListener::from_std(socket.into()))
}

fn main() -> io::Result<()> {
    let core_ids = core_affinity::get_core_ids().unwrap();
    println!("Total logical cores: {}", core_ids.len());

    // 保持 8 个 P-Core 的策略
    let p_cores = 8;
    let worker_cores: Vec<_> = core_ids.into_iter().take(p_cores * 2).step_by(2).collect();

    println!(
        "Starting {} workers (Engine Wrapped)...",
        worker_cores.len()
    );

    let mut handles = Vec::new();

    for (i, core_id) in worker_cores.into_iter().enumerate() {
        let core_id = core_id.clone();

        let handle = thread::spawn(move || {
            // 1. 绑核
            if !core_affinity::set_for_current(core_id) {
                eprintln!("Worker {} failed to pin to core", i);
            }

            // 2. 创建 Listener (SO_REUSEPORT)
            let addr = "127.0.0.1:9000";
            let listener = match create_reuse_port_listener(addr) {
                Ok(l) => l,
                Err(e) => panic!("Worker {} bind failed: {}", i, e),
            };

            // 3. 初始化 Engine
            let handler = EchoHandler;
            let codec = RawCodec;
            let mut engine = Engine::new(listener, handler, codec).unwrap();

            // 4. 启动 Loop
            if let Err(e) = engine.run() {
                eprintln!("Worker {} engine failed: {}", i, e);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    Ok(())
}
