use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;

use bytes::BytesMut;

use std::io::Write;

#[derive(Clone, Copy)]
struct MyHandler {
    engine: Option<EngineHandler>,
}

impl EventHandler for MyHandler {
    type Job = Vec<u8>;

    fn on_open(&self, conn: &mut Connection) -> Action<Self::Job> {
        // println!("New Connect in {}", conn.gfd.event_loop_index());
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action<Self::Job> {
        if let Some(datas) = conn.next(None, cache) {
            let _ = conn.write(datas);
        }
        Action::None
    }

    fn on_close(&self, _ctx: &mut Connection) -> Action<Self::Job> {
        // println!("Close Connect");
        Action::None
    }
}

fn main() {
    let mut options = Options::builder()
        // .reuse_port(true)
        .reuse_addr(true)
        .num_event_loop(5)
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
