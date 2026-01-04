use crate::balancer::Balancer;
use crate::command::Command;
use crate::event_loop::EventLoopHandle;
use crate::handler::EventHandler;
use crate::listener::Listener;
use crate::socket_addr::NetworkAddress;

use mio::{Events, Interest, Poll, Token};
use std::io;

const ACCEPTOR_TOKEN: Token = Token(0);

pub(crate) struct Acceptor<H: EventHandler> {
    poll: Poll,
    listener: Listener,
    workers: Vec<EventLoopHandle<H>>,
    balancer: Balancer,
}

impl<H: EventHandler> Acceptor<H> {
    pub fn new(
        listener: Listener,
        workers: Vec<EventLoopHandle<H>>,
        balancer: Balancer,
    ) -> io::Result<Self> {
        let poll = Poll::new()?;
        Ok(Self {
            poll,
            listener,
            workers,
            balancer,
        })
    }

    pub(crate) fn run(&mut self) -> io::Result<()> {
        self.poll
            .registry()
            .register(&mut self.listener, ACCEPTOR_TOKEN, Interest::READABLE)?;

        let local_addr = self.listener.local_addr()?;
        let mut events = Events::with_capacity(128);
        loop {
            if let Err(e) = self.poll.poll(&mut events, None) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }

            for event in events.iter() {
                match event.token() {
                    ACCEPTOR_TOKEN => self.accept_loop(&local_addr),
                    _ => {}
                }
            }
        }
    }

    fn accept_loop(&mut self, local_addr: &NetworkAddress) {
        loop {
            let (socket, peer_addr) = match self.listener.accept() {
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
                if let Err(_e) = handle.sender.send(cmd) {
                    eprintln!("Worker {} queue full, dropping connection", idx);
                    continue;
                }
                let _ = handle.waker.wake();
            }
        }
    }
}
