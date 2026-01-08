use bytes::BytesMut;
use rnet::command::Command;
use rnet::connection::Connection;
use rnet::engine::EngineBuilder;
use rnet::gfd::Gfd;
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;
use rnet::worker;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct ChatRoomHandler {
    clients: Arc<Mutex<HashSet<Gfd>>>,
}

impl ChatRoomHandler {
    fn new() -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl EventHandler for ChatRoomHandler {
    fn on_open(&self, conn: &mut Connection) -> Action {
        println!("New user connected: {:?}", conn.peer_addr());
        self.clients.lock().unwrap().insert(conn.gfd());
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action {
        loop {
            let mut found_len = None;
            // Peek data to find a newline
            if let Some(data) = conn.peek(None, cache) {
                if let Some(pos) = data.iter().position(|&b| b == b'\n') {
                    found_len = Some(pos + 1);
                }
            }

            if let Some(len) = found_len {
                // Consume the line
                if let Some(data) = conn.znext(Some(len), cache) {
                    let msg = data.to_vec();
                    let sender_gfd = conn.gfd();

                    // Broadcast to other clients
                    let clients = self.clients.lock().unwrap();
                    for &client_gfd in clients.iter() {
                        if client_gfd != sender_gfd {
                            let msg_clone = msg.clone();
                            worker::submit(client_gfd, move || {
                                Command::AsyncWrite(client_gfd, msg_clone)
                            });
                        }
                    }
                }
            } else {
                // No complete line found
                break;
            }
        }
        Action::None
    }

    fn on_close(&self, conn: &mut Connection) -> Action {
        println!("User disconnected: {:?}", conn.peer_addr());
        self.clients.lock().unwrap().remove(&conn.gfd());
        Action::None
    }
}

fn main() {
    let mut options = Options::builder()
        .reuse_addr(true)
        .num_event_loop(4)
        .build();

    let addrs = vec!["tcp://127.0.0.1:8888"];
    let net_socket_addrs = options.normalize(&addrs).unwrap();

    let (mut engine, _handler) = EngineBuilder::builder()
        .address(net_socket_addrs)
        .build(options, |_| ChatRoomHandler::new());

    println!("Chat room server running on tcp://127.0.0.1:8888");
    engine.run().expect("run failed");
}
