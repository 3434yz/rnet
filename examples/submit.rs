use rnet::command::Command;
use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;
use rnet::worker;

use bytes::BytesMut;

use std::io::Write;
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
    fn on_open(&self, conn: &mut Connection) -> Action {
        // println!("New Connect in {}", conn.gfd.event_loop_index());
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action {
        if let Some(datas) = conn.znext(None, cache) {
            let _ = conn.write(datas);
        }
        let gfd = conn.gfd();
        if let Some(engine) = &self.engine {
            let engine = engine.clone();
            worker::submit(move || {
                engine.send_command(gfd, Command::AsyncWrite(gfd, MyHandler::something()));
            });
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

    let (mut engine, _handler_copy) =
        EngineBuilder::builder()
            .address(net_socket_addrs)
            .build(options, |engine_handler| MyHandler {
                engine: Some(engine_handler),
            });

    engine.run().expect("run failed");
}
