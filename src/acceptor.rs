use crate::balancer::Balancer;
use crate::command::Command;
use crate::event_loop::EventLoopHandle;
use crate::listener::Listener;

use mio::{Events, Interest, Poll, Token};
use std::{io, sync::Arc};

pub(crate) struct AcceptorHandle {}

pub(crate) struct Acceptor {
    poll: Poll,
    listeners: Vec<Listener>,
    workers: Vec<Arc<EventLoopHandle>>,
    balancer: Balancer,
}

impl Acceptor {
    pub fn new(
        listeners: Vec<Listener>,
        workers: Vec<Arc<EventLoopHandle>>,
        balancer: Balancer,
    ) -> io::Result<Self> {
        let poll = Poll::new()?;
        Ok(Self {
            poll,
            listeners,
            workers,
            balancer,
        })
    }

    pub(crate) fn run(&mut self) -> io::Result<()> {
        for (idx, listener) in self.listeners.iter_mut().enumerate() {
            self.poll
                .registry()
                .register(listener, Token(idx), Interest::READABLE)?;
        }

        let mut events = Events::with_capacity(128);
        loop {
            if let Err(e) = self.poll.poll(&mut events, None) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }

            for event in events.iter() {
                let token = event.token();
                if token.0 < self.listeners.len() {
                    self.accept_loop(token.0);
                }
            }
        }
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
