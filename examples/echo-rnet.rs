use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;

use clap::Parser;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "echo-rnet", version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 9000)]
    port: u16,

    #[arg(short, long, default_value_t = 8)]
    loops: usize,
}

#[derive(Clone)]
struct MyHandler {
    _engine: Option<Arc<EngineHandler>>,
}

impl EventHandler for MyHandler {
    fn init(engine: Arc<EngineHandler>, _options: Arc<Options>) -> (Self, Action) {
        (
            Self {
                _engine: Some(engine),
            },
            Action::None,
        )
    }

    fn on_traffic(&self, conn: &mut Connection) -> Action {
        if let Some(data) = conn.next_vectored(None) {
            let bufs: Vec<_> = data.into_iter().map(|bs| bs.freeze()).collect();
            let _ = conn.write_vectored_bytes(&bufs);
        }
        Action::None
    }
}

fn main() {
    let args = Args::parse();

    let port = args.port;
    let num_event_loop = args.loops;

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

    let (mut engine, _handler_copy) = EngineBuilder::builder()
        .address(net_socket_addrs)
        .build::<MyHandler>(options)
        .unwrap();

    engine.run().expect("run failed");
}
