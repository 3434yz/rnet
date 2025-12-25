use rnet::connection::Connection;
use rnet::event_loop::EventLoop;
use rnet::handler::{Action, Context, EventHandler};
use rnet::worker::start_worker;

use bytes::BytesMut;
use core_affinity;

use rnet::listener::Listener;
use rnet::options::Options;
use std::io::{self, Write};
use std::sync::Arc;
use std::thread;

#[derive(Clone, Copy)]
struct MyHandler;

impl EventHandler for MyHandler {
    type Job = BytesMut;

    fn on_open(&self, _ctx: &mut Connection) -> Action<Self::Job> {
        println!("New Connect");
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action<Self::Job> {
        if let Some(datas) = conn.next(None, cache) {
            let _ = conn.write(datas);
        }
        Action::None
    }

    fn on_close(&self, _ctx: &mut Context) -> Action<Self::Job> {
        println!("Close Connect");
        Action::None
    }
}

// 保持高性能的 Listener 创建逻辑
// fn create_reuse_port_listener(addr_str: &str) -> io::Result<TcpListener> {
//     let addr: SocketAddr = addr_str.parse().unwrap();
//     let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;

//     socket.set_reuse_address(true)?;
//     #[cfg(unix)]
//     socket.set_reuse_port(true)?;
//     socket.set_nonblocking(true)?;

//     // 保持 4MB Buffer 优化
//     let _ = socket.set_recv_buffer_size(4 * 1024 * 1024);
//     let _ = socket.set_send_buffer_size(4 * 1024 * 1024);

//     socket.bind(&addr.into())?;
//     socket.listen(10240)?;

//     Ok(TcpListener::from_std(socket.into()))
// }

struct EventLoopInitCtx {
    event_loop: EventLoop<MyHandler>,
    core_id: core_affinity::CoreId,
}

fn main() -> io::Result<()> {
    let core_ids = core_affinity::get_core_ids().unwrap();
    println!("Total logical cores: {}", core_ids.len());

    // 保持 8 个 P-Core 的策略
    let p_cores = 8;
    let worker_cores: Vec<_> = core_ids.into_iter().take(p_cores * 2).step_by(2).collect();

    println!(
        "Starting {} workers (EventLoop Wrapped)...",
        worker_cores.len()
    );

    let mut event_loop_handles = Vec::new();
    let (req_tx, req_rx) = crossbeam::channel::unbounded();
    let mut event_loop_registry = Vec::new();
    let mut pending_event_loops = Vec::new();

    let mut options = Options::builder()
        .reuse_port(true)
        .reuse_addr(true)
        .multicore(true)
        .build();
    let addrs = vec!["tcp://127.0.0.1:9000"];
    let net_socket_addrs = options.normalize(&addrs).unwrap();
    let options = Arc::new(options);

    for (i, core_id) in worker_cores.into_iter().enumerate() {
        let (resp_tx, resp_rx) = crossbeam::channel::unbounded();

        let listener = Listener::bind(net_socket_addrs[0].clone(), options.clone())?;
        let handler = MyHandler;

        let event_loop = EventLoop::new(i, listener, handler, req_tx.clone(), resp_rx)?;
        event_loop_registry.push((resp_tx, event_loop.get_waker()));
        pending_event_loops.push(EventLoopInitCtx {
            event_loop: event_loop,
            core_id,
        });
    }

    println!("Starting 4 Workers...");
    for i in 0..4 {
        let req_rx = req_rx.clone();
        let registry = event_loop_registry.clone();
        start_worker(i, req_rx, registry, |job| -> Vec<u8> {
            thread::sleep(std::time::Duration::from_secs(5));
            // let response_str = format!("Processed: {:?}", job);
            // response_str.into_bytes()
            job.to_vec()
        });
    }

    for (i, ctx) in pending_event_loops.into_iter().enumerate() {
        let mut event_loop = ctx.event_loop;
        let core_id = ctx.core_id;

        let handle = thread::spawn(move || {
            if !core_affinity::set_for_current(core_id) {
                eprintln!("EventLoop {} failed to pin to core", i);
            }
            if let Err(e) = event_loop.run() {
                eprintln!("EventLoop {} failed: {}", i, e);
            }
        });
        event_loop_handles.push(handle);
    }

    drop(event_loop_registry);

    for handle in event_loop_handles {
        handle.join().unwrap();
    }

    Ok(())
}
