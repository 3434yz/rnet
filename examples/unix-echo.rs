use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;
use std::sync::Arc;

#[derive(Clone)]
struct UnixEchoHandler;

impl EventHandler for UnixEchoHandler {
    fn init(_engine: Arc<EngineHandler>, _options: Arc<Options>) -> (Self, Action) {
        (UnixEchoHandler, Action::None)
    }

    fn on_open(&self, conn: &mut Connection) -> Action {
        println!("New connection: {:?}", conn.peer_addr());
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection) -> Action {
        if let Some(data) = conn.next_contiguous(None) {
            let _ = conn.write_bytes(data.freeze());
        }
        Action::None
    }

    fn on_close(&self, conn: &mut Connection) -> Action {
        println!("Connection closed: {:?}", conn.peer_addr());
        Action::None
    }
}

fn main() {
    let mut options = Options::builder().num_event_loop(2).build();

    let addrs = vec!["unix:///tmp/rnet-echo.sock"];

    let net_socket_addrs = match options.normalize(&addrs) {
        Ok(addrs) => addrs,
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        }
    };

    let (mut engine, _handler) = EngineBuilder::builder()
        .address(net_socket_addrs)
        .build::<UnixEchoHandler>(options)
        .unwrap();

    println!("Unix Echo Server running on unix:///tmp/rnet-echo.sock");
    engine.run().expect("run failed");
}
