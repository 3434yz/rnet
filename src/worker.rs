use crossbeam::channel::{Receiver, Sender, bounded};
use std::sync::OnceLock;
use std::thread;

// 1. 改为存储一组 Senders
static GLOBAL_SENDERS: OnceLock<Vec<Sender<Task>>> = OnceLock::new();

pub struct Task {
    pub closure: Box<dyn FnOnce() + Send>,
}

pub fn submit<F>(f: F, shard_idx: usize)
where
    F: FnOnce() + Send + 'static,
{
    if let Some(senders) = GLOBAL_SENDERS.get() {
        let task = Task {
            closure: Box::new(f),
        };
        let idx = shard_idx % senders.len();
        let _ = senders[idx].send(task);
    } else {
        eprintln!("WorkerPool not initialized");
    }
}

pub struct WorkerPool {}

impl WorkerPool {
    pub fn new(worker_count: usize) -> Self {
        let mut senders: Vec<Sender<Task>> = Vec::with_capacity(worker_count);

        for id in 0..worker_count {
            let (tx, rx) = bounded(10000);
            senders.push(tx);

            thread::spawn(move || {
                while let Ok(task) = rx.recv() {
                    (task.closure)();

                    while let Ok(next_task) = rx.try_recv() {
                        (next_task.closure)();
                    }
                }
                println!("Worker {} stopped.", id);
            });
        }

        if GLOBAL_SENDERS.set(senders).is_err() {
            eprintln!("Warning: WorkerPool initialized twice");
        }

        Self {}
    }
}
