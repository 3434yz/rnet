use rnet::command::Command;
use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;

use bytes::BytesMut;
use rnet::worker;

use std::sync::Arc;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Clone)]
struct MyHandler {
    engine: Option<Arc<EngineHandler>>,
}

impl EventHandler for MyHandler {
    fn on_open(&self, conn: &mut Connection) -> Action {
        // println!("New Connect in {}", conn.gfd.event_loop_index());
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action {
        // let start_time = Instant::now();
        if let Some(datas) = conn.next(None, cache) {
            let datas = datas.freeze();
            let gfd = conn.gfd();
            if let Some(engine) = &self.engine {
                let engine = engine.clone();
                worker::submit(
                    move || {
                        engine.send_command(gfd, Command::AsyncWrite(gfd, datas));
                    },
                    gfd.event_loop_index(),
                );
            }
        }
        Action::None
    }

    fn on_close(&self, _ctx: &mut Connection) -> Action {
        // println!("Close Connect");
        Action::None
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut port: u16 = 0;
    let mut num_event_loop: usize = 0;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().expect("Invalid port number");
                    i += 1;
                }
            }
            "--loops" => {
                if i + 1 < args.len() {
                    num_event_loop = args[i + 1].parse().expect("Invalid num_event_loop");
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    // 校验必填参数
    if port == 0 || num_event_loop == 0 {
        eprintln!("Usage: echo --port <port> --loops <num_event_loop>");
        std::process::exit(1);
    }

    let mut options = Options::builder()
        .reuse_addr(true)
        .num_event_loop(num_event_loop)
        .build();

    let addrs = vec![format!("tcp://127.0.0.1:{}", port)];
    let net_socket_addrs = options.normalize(&addrs).unwrap();

    let (mut engine, _handler_copy) =
        EngineBuilder::builder()
            .address(net_socket_addrs)
            .build(options, |engine_handler| MyHandler {
                engine: Some(engine_handler),
            });

    engine.run().expect("run failed");
}
