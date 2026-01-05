use mio::event::Source;
use mio::{Events, Interest, Poll, Registry, Token, Waker as MioWaker};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct Poller {
    poll: Poll,
}

impl Poller {
    pub fn new() -> io::Result<Self> {
        Ok(Self { poll: Poll::new()? })
    }

    pub fn registry(&self) -> &Registry {
        self.poll.registry()
    }

    pub fn poll(&mut self, events: &mut Events, timeout: Option<Duration>) -> io::Result<()> {
        self.poll.poll(events, timeout)
    }

    pub fn register<S>(&self, source: &mut S, token: Token) -> io::Result<()>
    where
        S: Source + ?Sized,
    {
        self.poll
            .registry()
            .register(source, token, Interest::READABLE)
    }

    pub fn enable_write<S>(&self, source: &mut S, token: Token) -> io::Result<()>
    where
        S: Source + ?Sized,
    {
        self.poll
            .registry()
            .reregister(source, token, Interest::READABLE | Interest::WRITABLE)
    }

    pub fn disable_write<S>(&self, source: &mut S, token: Token) -> io::Result<()>
    where
        S: Source + ?Sized,
    {
        self.poll
            .registry()
            .reregister(source, token, Interest::READABLE)
    }

    pub fn deregister<S>(&self, source: &mut S) -> io::Result<()>
    where
        S: Source + ?Sized,
    {
        self.poll.registry().deregister(source)
    }
}

#[derive(Debug)]
pub struct Waker {
    waker: MioWaker,
    awoken: AtomicBool,
}

impl Waker {
    pub fn new(registry: &Registry, token: Token) -> io::Result<Self> {
        let waker = MioWaker::new(registry, token)?;
        Ok(Self {
            waker,
            awoken: AtomicBool::new(false),
        })
    }

    pub fn wake(&self) -> io::Result<()> {
        if !self.awoken.swap(true, Ordering::SeqCst) {
            self.waker.wake()?;
        }
        Ok(())
    }

    pub fn reset(&self) {
        self.awoken.store(false, Ordering::SeqCst);
    }
}
