use bytes::Bytes;
use rnet::command::Command;
use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::gfd::Gfd;
use rnet::handler::{Action, EventHandler, TickContext};
use rnet::options::Options;
use rnet::worker;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct ChatRoomHandler {
    engine: Option<Arc<EngineHandler>>,
    clients: Arc<Vec<Mutex<HashSet<Gfd>>>>,
}

impl ChatRoomHandler {
    fn new(engine: Arc<EngineHandler>, options: Arc<Options>) -> Self {
        let mut clients = Vec::with_capacity(options.num_event_loop);
        for _ in 0..options.num_event_loop {
            clients.push(Mutex::new(HashSet::new()));
        }
        Self {
            engine: Some(engine),
            clients: Arc::new(clients),
        }
    }
}

impl EventHandler for ChatRoomHandler {
    fn init(engine: Arc<EngineHandler>, options: Arc<Options>) -> (Self, Action) {
        (Self::new(engine, options), Action::None)
    }

    fn on_open(&self, conn: &mut Connection) -> Action {
        println!("New user connected: {:?}", conn.peer_addr());
        let idx = conn.gfd().event_loop_index();
        self.clients[idx].lock().unwrap().insert(conn.gfd());
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection) -> Action {
        loop {
            let mut found_len = None;
            // Peek data to find a newline
            if let Some(data) = conn.peek(None) {
                if let Some(pos) = data.iter().position(|&b| b == b'\n') {
                    found_len = Some(pos + 1);
                }
            }

            let Some(len) = found_len else {
                break;
            };
            if let Some(data) = conn.next_contiguous(Some(len)) {
                let msg = data.freeze();
                let sender_gfd = conn.gfd();
                let clients = self.clients.clone();
                let engine = self.engine.clone().unwrap();

                worker::submit(
                    move || {
                        // 遍历所有分片进行广播
                        for shard in clients.iter() {
                            let guard = shard.lock().unwrap();
                            for &client_gfd in guard.iter() {
                                if client_gfd != sender_gfd {
                                    engine.send_command(
                                        client_gfd,
                                        Command::AsyncWrite(client_gfd, msg.clone()),
                                    );
                                }
                            }
                        }
                    },
                    sender_gfd.event_loop_index(),
                );
            }
        }
        Action::None
    }

    fn on_close(&self, conn: &mut Connection) -> Action {
        println!("User disconnected: {:?}", conn.peer_addr());
        let idx = conn.gfd().event_loop_index();
        self.clients[idx].lock().unwrap().remove(&conn.gfd());
        conn.close();
        Action::None
    }

    fn on_tick(&self, ctx: &mut TickContext) -> (std::time::Duration, Action) {
        let msg = Bytes::from("Server heartbeat\n");
        for conn in ctx.connections() {
            let _ = conn.write_bytes(msg.clone());
        }
        (std::time::Duration::from_secs(5), Action::None)
    }
}

fn main() {
    let mut options = Options::builder()
        .reuse_addr(true)
        .num_event_loop(4)
        .ticker(true)
        .build();

    let addrs = vec!["tcp://127.0.0.1:8888"];
    let net_socket_addrs = options.normalize(&addrs).unwrap();

    let (mut engine, _handler) = EngineBuilder::builder()
        .address(net_socket_addrs)
        .build::<ChatRoomHandler>(options)
        .unwrap();

    println!("Chat room server running on tcp://127.0.0.1:8888");
    engine.run().expect("run failed");
}
