use crate::command::Command;
use crate::event_loop::EventLoopWaker;
use crossbeam::channel::{Receiver, Sender};
use std::sync::Arc;
use std::thread;

pub fn start_worker<J, F>(
    id: usize,
    task_receiver: Receiver<Command<J>>,
    engine_registry: Vec<(Sender<Command<J>>, Arc<EventLoopWaker>)>,
    processor: F,
) where
    J: Send + 'static,
    F: Fn(J) -> Vec<u8> + Send + Sync + 'static,
{
    let registry = engine_registry;
    thread::spawn(move || {
        while let Ok(command) = task_receiver.recv() {
            if let Command::JobReq(gfd, job) = command {
                let index = gfd.event_loop_index();
                let data = processor(job);
                let response = Command::JobResp(gfd, data);

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
        }

        println!("Worker {} stopped.", id);
    });
}
