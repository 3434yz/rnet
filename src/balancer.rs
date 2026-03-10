use crate::event_loop::EventLoopHandle;
use crate::options::LoadBalancing;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
pub trait LoadBalancer: Send + Sync {
    fn select(&self, workers: &[Arc<EventLoopHandle>]) -> usize;
}

pub struct Balancer {
    inner: Box<dyn LoadBalancer>,
}

impl Balancer {
    pub fn new(option: LoadBalancing) -> Self {
        let inner: Box<dyn LoadBalancer> = match option {
            LoadBalancing::RoundRobin => Box::new(RoundRobin::default()),
            LoadBalancing::LeastConnections => Box::new(LeastConnections::default()),
            LoadBalancing::SourceAddrHash => todo!(),
        };
        Self { inner }
    }

    pub fn select(&self, workers: &[Arc<EventLoopHandle>]) -> usize {
        self.inner.select(workers)
    }
}

pub struct RoundRobin {
    next: AtomicUsize,
}

impl RoundRobin {
    pub fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
        }
    }
}

impl Default for RoundRobin {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadBalancer for RoundRobin {
    fn select(&self, workers: &[Arc<EventLoopHandle>]) -> usize {
        if workers.is_empty() {
            return 0;
        }
        let current = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        current % workers.len()
    }
}

pub struct LeastConnections;

impl Default for LeastConnections {
    fn default() -> Self {
        Self
    }
}

impl LoadBalancer for LeastConnections {
    fn select(&self, workers: &[Arc<EventLoopHandle>]) -> usize {
        if workers.is_empty() {
            return 0;
        }
        workers
            .iter()
            .enumerate()
            .min_by_key(|(_, handle)| handle.connection_count())
            .map(|(idx, _)| idx)
            .unwrap_or(0)
    }
}
