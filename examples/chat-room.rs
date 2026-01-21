use rnet::command::Command;
use rnet::connection::Connection;
use rnet::engine::{EngineBuilder, EngineHandler};
use rnet::gfd::Gfd;
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;
use rnet::worker;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct ChatRoomHandler {
    engine: Option<Arc<EngineHandler>>,
    clients: Arc<Mutex<HashSet<Gfd>>>,
}

impl ChatRoomHandler {
    fn new(engine: Arc<EngineHandler>) -> Self {
        Self {
            engine: Some(engine),
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
            if let Some(data) = conn.next(Some(len)) {
                let msg = data.freeze();
                let sender_gfd = conn.gfd();

                let clients_guard = self.clients.lock().unwrap();
                let target_clients: Vec<Gfd> = clients_guard
                    .iter()
                    .filter(|&&c| c != sender_gfd)
                    .cloned()
                    .collect();
                drop(clients_guard); // 尽早释放锁

                let engine = self.engine.clone().unwrap();
                worker::submit(
                    move || {
                        for client_gfd in target_clients {
                            engine.send_command(
                                client_gfd,
                                Command::AsyncWrite(client_gfd, msg.clone()),
                            );
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
        .build(options, |engine| ChatRoomHandler::new(engine));

    println!("Chat room server running on tcp://127.0.0.1:8888");
    engine.run().expect("run failed");
}
