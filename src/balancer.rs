use crate::event_loop::EventLoopHandle;
use crate::handler::EventHandler;
use crate::options::LoadBalancing;
use std::sync::atomic::AtomicUsize;
pub trait LoadBalancer: Send + Sync {
    fn select<H: EventHandler>(&self, workers: &[EventLoopHandle<H>]) -> usize;
}

pub enum Balancer {
    RoundRobin(RoundRobin),
    LeastConnections(LeastConnections),
}

impl Balancer {
    pub fn new(option: LoadBalancing) -> Self {
        match option {
            LoadBalancing::RoundRobin => Self::RoundRobin(RoundRobin::default()),
            LoadBalancing::LeastConnections => Self::LeastConnections(LeastConnections::default()),
            LoadBalancing::SourceAddrHash => todo!(),
        }
    }

    pub fn select<H: EventHandler>(&self, workers: &[EventLoopHandle<H>]) -> usize {
        match self {
            Self::RoundRobin(inner) => inner.select(workers),
            Self::LeastConnections(inner) => inner.select(workers),
        }
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
    fn select<H: EventHandler>(&self, workers: &[EventLoopHandle<H>]) -> usize {
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
    fn select<H: EventHandler>(&self, workers: &[EventLoopHandle<H>]) -> usize {
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
