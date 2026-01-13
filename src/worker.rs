use crossbeam::channel::{Receiver, Sender, unbounded};
use std::sync::OnceLock;
use std::thread;

static GLOBAL_SENDER: OnceLock<Sender<Task>> = OnceLock::new();

pub struct Task {
    closure: Box<dyn FnOnce() + Send>,
}

pub fn submit<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    if let Some(sender) = GLOBAL_SENDER.get() {
        let task = Task {
            closure: Box::new(f),
        };
        let _ = sender.send(task);
    } else {
        eprintln!("WorkerPool not initialized, task dropped");
    }
}

pub struct WorkerPool {
    receiver: Receiver<Task>,
}

impl WorkerPool {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        if GLOBAL_SENDER.set(tx).is_err() {
            eprintln!("Warning: WorkerPool initialized more than once");
        }
        Self { receiver: rx }
    }

    pub fn run(self, count: usize) {
        let receiver = self.receiver;

        for id in 0..count {
            let rx = receiver.clone();
            thread::spawn(move || {
                while let Ok(task) = rx.recv() {
                    (task.closure)();
                }
                println!("Worker {} stopped.", id);
            });
        }
    }
}
