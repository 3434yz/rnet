use crossbeam::channel::{Sender, bounded};
use std::sync::RwLock; // 使用读写锁替代 OnceLock
use std::thread::{self, JoinHandle};

pub struct Task {
    pub closure: Box<dyn FnOnce() + Send>,
}

static GLOBAL_SENDERS: RwLock<Option<Vec<Sender<Task>>>> = RwLock::new(None);

pub fn submit<F>(f: F, shard_idx: usize)
where
    F: FnOnce() + Send + 'static,
{
    let lock = GLOBAL_SENDERS.read().unwrap();

    if let Some(senders) = lock.as_ref() {
        let task = Task {
            closure: Box::new(f),
        };
        let idx = shard_idx % senders.len();

        let _ = senders[idx].send(task);
    } else {
        eprintln!("WorkerPool is not running or has been shut down.");
    }
}

pub struct WorkerPool {
    workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    pub fn new(worker_count: usize) -> Self {
        let mut senders: Vec<Sender<Task>> = Vec::with_capacity(worker_count);
        let mut workers: Vec<JoinHandle<()>> = Vec::with_capacity(worker_count);

        for id in 0..worker_count {
            let (tx, rx) = bounded(10000);
            senders.push(tx);

            let handle = thread::spawn(move || {
                while let Ok(task) = rx.recv() {
                    (task.closure)();
                    while let Ok(next_task) = rx.try_recv() {
                        (next_task.closure)();
                    }
                }
                println!("Worker {} channel disconnected, stopping.", id);
            });
            workers.push(handle);
        }

        let mut lock = GLOBAL_SENDERS.write().unwrap();
        if lock.is_none() {
            *lock = Some(senders);
        } else {
            eprintln!("Warning: WorkerPool initialized twice");
        }

        Self { workers }
    }

    pub fn shutdown(self) {
        println!("Shutting down worker pool...");
        {
            let mut lock = GLOBAL_SENDERS.write().unwrap();

            let _dropped_senders = lock.take();

            println!("Global senders dropped. Signaling workers implicitly.");
        }

        for worker in self.workers {
            let _ = worker.join();
        }
        println!("All workers stopped.");
    }
}
