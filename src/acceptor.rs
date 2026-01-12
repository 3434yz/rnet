use crate::command::Command;
use crate::event_loop::EventLoopHandle;
use crate::listener::Listener;
use crate::{balancer::Balancer, poller::Waker};

use mio::{Events, Interest, Poll};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{io, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct AcceptorHandle {
    waker: Arc<Waker>,
    is_running: Arc<AtomicBool>,
}

impl AcceptorHandle {
    pub(crate) fn shutdown(&self) {
        self.is_running.store(false, Ordering::SeqCst);
        let _ = self.waker.wake();
    }
}

pub(crate) struct Acceptor {
    poll: Poll,
    waker: Arc<Waker>,
    listeners: Vec<Listener>,
    workers: Vec<Arc<EventLoopHandle>>,
    balancer: Balancer,
    is_running: Arc<AtomicBool>,
}

impl Acceptor {
    pub fn new(
        listeners: Vec<Listener>,
        workers: Vec<Arc<EventLoopHandle>>,
        balancer: Balancer,
    ) -> io::Result<(Self, AcceptorHandle)> {
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry())?);
        let is_running = Arc::new(AtomicBool::new(true));

        let handle = AcceptorHandle {
            waker: waker.clone(),
            is_running: is_running.clone(),
        };

        Ok((
            Self {
                poll,
                waker,
                listeners,
                workers,
                balancer,
                is_running,
            },
            handle,
        ))
    }

    pub(crate) fn run(&mut self) -> io::Result<()> {
        for (idx, listener) in self.listeners.iter_mut().enumerate() {
            self.poll.registry().register(
                listener,
                crate::poller::listener_token(idx),
                Interest::READABLE,
            )?;
        }

        let mut events = Events::with_capacity(128);
        loop {
            if !self.is_running.load(Ordering::SeqCst) {
                break;
            }

            if let Err(e) = self.poll.poll(&mut events, None) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }

            for event in events.iter() {
                let token = event.token();
                if token == crate::poller::WAKE_TOKEN {
                    if !self.is_running.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    self.waker.reset();
                } else if let Some(idx) = crate::poller::is_listener_token(token) {
                    self.accept_loop(idx);
                }
            }
        }
        Ok(())
    }

    fn accept_loop(&self, index: usize) {
        let listener = &self.listeners[index];
        let local_addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(e) => {
                eprintln!("Failed to get local addr: {}", e);
                return;
            }
        };

        loop {
            let (socket, peer_addr) = match listener.accept() {
                Ok(res) => res,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    eprintln!("Accept error: {}", e);
                    break;
                }
            };

            let idx = self.balancer.select(&self.workers);

            if let Some(handle) = self.workers.get(idx) {
                let cmd = Command::Register(socket, local_addr.clone(), peer_addr);
                if let Err(_e) = handle.urgent_sender.send(cmd) {
                    eprintln!("Worker {} queue full, dropping connection", idx);
                    continue;
                }
                let _ = handle.waker.wake();
            }
        }
    }
}
