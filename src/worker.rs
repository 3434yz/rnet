use crate::command::Command;
use crate::event_loop::EventLoopHandle;
use crate::gfd::Gfd;
use crossbeam::channel::{Receiver, Sender, unbounded};
use std::sync::OnceLock;
use std::thread;

static GLOBAL_SENDER: OnceLock<Sender<Task>> = OnceLock::new();

pub struct Task {
    gfd: Gfd,
    closure: Box<dyn FnOnce() -> Command + Send>,
}

pub fn submit<F>(gfd: Gfd, f: F)
where
    F: FnOnce() -> Command + Send + 'static,
{
    if let Some(sender) = GLOBAL_SENDER.get() {
        let task = Task {
            gfd,
            closure: Box::new(f),
        };
        let _ = sender.send(task);
    } else {
        eprintln!("WorkerPool not initialized, task dropped");
    }
}

pub struct WorkerPool {
    registry: Vec<EventLoopHandle>,
    receiver: Receiver<Task>,
}

impl WorkerPool {
    pub fn new(registry: Vec<EventLoopHandle>) -> Self {
        let (tx, rx) = unbounded();
        if GLOBAL_SENDER.set(tx).is_err() {
            eprintln!("Warning: WorkerPool initialized more than once");
        }
        Self {
            registry,
            receiver: rx,
        }
    }

    pub fn run(self, count: usize) {
        let receiver = self.receiver;
        let registry = self.registry;

        for id in 0..count {
            let rx = receiver.clone();
            let reg = registry.clone();
            thread::spawn(move || {
                while let Ok(task) = rx.recv() {
                    let command = (task.closure)();
                    if let Command::None = command {
                        continue;
                    }

                    let gfd = task.gfd;
                    let index = gfd.event_loop_index();
                    let priority = command.priority();
                    if let Some(handle) = reg.get(index) {
                        if let Err(e) = handle.trigger(priority, command) {
                            eprintln!("Worker {}: Failed to send response: {}", id, e);
                            continue;
                        }
                    }
                }
                println!("Worker {} stopped.", id);
            });
        }
    }
}
