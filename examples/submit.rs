use rnet::command::Command;
use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;
use rnet::worker;

use bytes::Bytes;

use std::sync::Arc;

#[derive(Clone)]
struct MyHandler {
    engine: Option<Arc<EngineHandler>>,
}

impl MyHandler {
    fn something() -> Vec<u8> {
        b"HelloWorld"[..].to_vec()
    }
}

impl EventHandler for MyHandler {
    fn init(engine: Arc<EngineHandler>) -> (Self, Action) {
        (
            Self {
                engine: Some(engine),
            },
            Action::None,
        )
    }

    fn on_open(&self, _conn: &mut Connection) -> Action {
        // println!("New Connect in {}", conn.gfd.event_loop_index());
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection) -> Action {
        if let Some(datas) = conn.next_contiguous(None) {
            let _ = conn.write_bytes(datas.freeze());
        }

        if let Some(engine) = &self.engine {
            let gfd = conn.gfd();
            let engine = engine.clone();
            worker::submit(
                move || {
                    let datas = Bytes::from(MyHandler::something());
                    engine.send_command(gfd, Command::AsyncWrite(gfd, datas));
                },
                gfd.event_loop_index(),
            );
        }
        Action::None
    }

    fn on_close(&self, _ctx: &mut Connection) -> Action {
        // println!("Close Connect");
        Action::None
    }
}

fn main() {
    let mut options = Options::builder()
        // .reuse_port(true)
        .reuse_addr(true)
        .num_event_loop(8)
        .build();

    let addrs = vec!["tcp://127.0.0.1:9000"];
    let net_socket_addrs = options.normalize(&addrs).unwrap();

    let (mut engine, _handler_copy) = EngineBuilder::builder()
        .address(net_socket_addrs)
        .build::<MyHandler>(options)
        .unwrap();

    engine.run().expect("run failed");
}
