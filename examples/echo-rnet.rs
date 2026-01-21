use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;

use std::io::Write;
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

    fn on_traffic(&self, conn: &mut Connection) -> Action {
        if let Some(data) = conn.next(None) {
            let _ = conn.write(&data);
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

    let mut port: u16 = 9000;
    let mut num_event_loop: usize = 8;

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
            "--help" | "-h" => {
                println!("Usage: echo [--port <port>] [--loops <num_event_loop>]");
                println!("Defaults: port=9000, loops=8");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    println!(
        "Starting server on port: {}, loops: {}",
        port, num_event_loop
    );

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
