use crate::command::{Request, Response};
use crossbeam::channel::{Receiver, Sender};
use mio::Waker;
use std::sync::Arc;
use std::thread;

pub fn start_worker<J, F>(
    id: usize,
    task_receiver: Receiver<Request<J>>,
    engine_registry: Vec<(Sender<Response>, Arc<Waker>)>,
    processor: F,
) where
    J: Send + 'static,
    F: Fn(J) -> Vec<u8> + Send + Sync + 'static,
{
    let registry = engine_registry;
    thread::spawn(move || {
        while let Ok(req) = task_receiver.recv() {
            let index = req.gfd.event_loop_index();
            let data = processor(req.job);
            let response = Response { gfd: req.gfd, data };

            if let Some((resp_sender, waker)) = registry.get(index) {
                if let Err(e) = resp_sender.send(response) {
                    eprintln!(
                        "Worker {}: Failed to send response to EventLoop {}: {}",
                        id, index, e
                    );
                    continue;
                }

                if let Err(e) = waker.wake() {
                    eprintln!("Worker {}: Failed to wake EventLoop {}: {}", id, index, e);
                }
            } else {
                eprintln!(
                    "Worker {}: Received task from unknown EventLoop ID {}",
                    id, index
                );
            }
        }

        println!("Worker {} stopped.", id);
    });
}
