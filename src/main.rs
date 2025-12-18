use bytes::BytesMut;
use core_affinity;
use mio::net::TcpListener;
use rnet::codec::RawCodec;
use rnet::engine::Engine;
use rnet::handler::{Action, Context, EventHandler};
use rnet::worker::start_worker;
use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::SocketAddr;
use std::thread;

#[derive(Clone, Copy)]
struct MyHandler;

impl EventHandler for MyHandler {
    type Message = BytesMut;
    type Job = BytesMut;

    fn on_open(&self, _ctx: &mut Context) -> Action<Self::Job> {
        println!("New Connect");
        Action::None
    }

    fn on_message(&self, ctx: &mut Context, in_buf: Self::Message) -> Action<Self::Job> {
        ctx.out_buf.extend_from_slice(&in_buf);
        Action::None
    }

    fn on_close(&self, _ctx: &mut Context) -> Action<Self::Job> {
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

struct EngineInitCtx<J> {
    engine: Engine<MyHandler, RawCodec, J>,
    core_id: core_affinity::CoreId,
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

    let mut engine_handles = Vec::new();
    let (req_tx, req_rx) = crossbeam::channel::unbounded();
    let mut engine_registry = Vec::new();
    let mut pending_engines = Vec::new();


    for (i, core_id) in worker_cores.into_iter().enumerate() {
        let (resp_tx, resp_rx) = crossbeam::channel::unbounded();

        let addr = "127.0.0.1:9000";
        let listener = match create_reuse_port_listener(addr) {
            Ok(l) => l,
            Err(e) => panic!("Worker {} bind failed: {}", i, e),
        };

        // 3. 初始化 Engine
        let handler = MyHandler;
        let codec = RawCodec;

        let engine = Engine::new(i, listener, handler, codec, req_tx.clone(), resp_rx)?;
        engine_registry.push((resp_tx, engine.get_waker()));
        pending_engines.push(EngineInitCtx { engine, core_id });
    }

    println!("Starting 4 Workers...");
    for i in 0..4 {
        let req_rx = req_rx.clone();
        let registry = engine_registry.clone();
        start_worker(i, req_rx, registry, |job| -> Vec<u8> {
            // thread::sleep(std::time::Duration::from_millis(100));
            // let response_str = format!("Processed: {:?}", job);
            // response_str.into_bytes()
            job.to_vec()
        });
    }

    for (i, ctx) in pending_engines.into_iter().enumerate() {
        let mut engine = ctx.engine;
        let core_id = ctx.core_id;

        let handle = thread::spawn(move || {
            if !core_affinity::set_for_current(core_id) {
                eprintln!("Engine {} failed to pin to core", i);
            }
            if let Err(e) = engine.run() {
                eprintln!("Engine {} failed: {}", i, e);
            }
        });
        engine_handles.push(handle);
    }

    drop(engine_registry);

    for handle in engine_handles {
        handle.join().unwrap();
    }

    Ok(())
}
