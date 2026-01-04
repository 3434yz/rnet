use crate::command::Command;
use crate::connection::Connection;
use crate::gfd::Gfd;
use crate::handler::{Action, EventHandler};
use crate::io_buffer::IOBuffer;
use crate::listener::Listener;
use crate::options::Options;
use crate::socket::Socket;
use crate::socket_addr::NetworkAddress;

use bytes::{Buf, BytesMut};
use crossbeam::channel::{Receiver, Sender, TryRecvError};
use mio::event::Event;
use mio::{Events, Interest, Poll, Token, Waker};
use slab::Slab;

use std::io::{self, Read, Write};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

const SERVER_TOKEN: Token = Token(usize::MAX);
const WAKE_TOKEN: Token = Token(usize::MAX - 1);
const DEFAULT_BUF_SIZE: usize = 64 * 1024;
const EVENTS_CAP: usize = 1024;

enum IoStatus {
    Completed,
    Yield,
    Closed(bool),
}

pub(crate) struct EventLoopBuilder {}

#[derive(Debug)]
pub struct EventLoopWaker {
    waker: Waker,
    awoken: AtomicBool,
}

impl EventLoopWaker {
    pub fn new(waker: Waker) -> Self {
        Self {
            waker,
            awoken: AtomicBool::new(false),
        }
    }

    pub fn wake(&self) -> io::Result<()> {
        if !self.awoken.swap(true, Ordering::SeqCst) {
            self.waker.wake()?;
        }
        Ok(())
    }

    pub(crate) fn reset(&self) {
        self.awoken.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Debug)]
pub struct EventLoopHandle<H: EventHandler> {
    pub idx: usize,
    pub sender: Sender<Command<H::Job>>,
    pub waker: Arc<EventLoopWaker>,
    pub conn_count: Arc<AtomicUsize>,
}

impl<H> EventLoopHandle<H>
where
    H: EventHandler,
{
    pub fn new(
        idx: usize,
        sender: Sender<Command<H::Job>>,
        waker: Arc<EventLoopWaker>,
        conn_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            idx,
            sender,
            waker,
            conn_count,
        }
    }

    pub fn connection_count(&self) -> usize {
        self.conn_count.load(Ordering::Relaxed)
    }
}

pub(crate) struct EventLoop<H>
where
    H: EventHandler,
{
    pub loop_id: u8,
    poll: Poll,
    waker: Arc<EventLoopWaker>,
    listener: Option<Listener>,
    options: Arc<Options>,
    connections: Slab<Connection>,
    handler: Arc<H>,
    buffer: Box<IOBuffer>,
    cache: BytesMut,
    job_sender: Sender<Command<H::Job>>,
    inner_sender: Sender<Command<H::Job>>,
    inner_receiver: Receiver<Command<H::Job>>,
    conn_count: Arc<AtomicUsize>,
}

impl<H> EventLoop<H>
where
    H: EventHandler,
{
    pub(crate) fn new(
        idx: u8,
        mut listener: Option<Listener>,
        options: Arc<Options>,
        handler: Arc<H>,
        job_sender: Sender<Command<H::Job>>,
        inner_sender: Sender<Command<H::Job>>,
        inner_receiver: Receiver<Command<H::Job>>,
        conn_count: Arc<AtomicUsize>,
    ) -> io::Result<Self> {
        let poll = Poll::new()?;
        let waker = Waker::new(poll.registry(), WAKE_TOKEN)?;
        let waker = Arc::new(EventLoopWaker::new(waker));

        if let Some(listener) = listener.as_mut() {
            poll.registry()
                .register(listener, SERVER_TOKEN, Interest::READABLE)?;
        }

        let buffer = Box::new(IOBuffer::new(DEFAULT_BUF_SIZE));
        let cache = BytesMut::with_capacity(DEFAULT_BUF_SIZE);
        let connections = Slab::with_capacity(4096);

        Ok(Self {
            loop_id: idx,
            poll,
            listener,
            connections,
            handler,
            buffer,
            cache,
            job_sender,
            inner_sender,
            inner_receiver,
            waker,
            conn_count,
            options,
        })
    }

    fn reregister(&mut self, key: usize) -> io::Result<()> {
        if let Some(conn) = self.connections.get_mut(key) {
            let mut interests = Interest::READABLE;
            if !conn.out_buf.is_empty() {
                interests |= Interest::WRITABLE;
            }
            self.poll
                .registry()
                .reregister(&mut conn.socket, Token(key), interests)?;
        }
        Ok(())
    }

    pub(crate) fn get_waker(&self) -> Arc<EventLoopWaker> {
        self.waker.clone()
    }

    pub(crate) fn run(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(EVENTS_CAP);
        loop {
            let timeout = if self.inner_receiver.is_empty() {
                None
            } else {
                Some(Duration::ZERO)
            };

            if let Err(e) = self.poll.poll(&mut events, timeout) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }

            for event in events.iter() {
                match event.token() {
                    SERVER_TOKEN => self.accept_loop()?,
                    WAKE_TOKEN => {
                        self.waker.reset();
                        self.process_command()?
                    }
                    token => self.process_io(token, &event)?,
                }
            }
        }
    }

    fn process_command(&mut self) -> io::Result<()> {
        loop {
            match self.inner_receiver.try_recv() {
                Ok(c) => match c {
                    Command::JobReq(gfd, job) => {
                        let req = Command::JobReq(gfd, job);
                        let _ = self.job_sender.send(req); // todo handler error
                    }
                    Command::JobResp(gfd, data) => {
                        let loop_idx = gfd.event_loop_index();
                        if loop_idx != self.loop_id as usize {
                            eprintln!("Error: Received response for wrong engine {}", loop_idx);
                            continue;
                        }
                        let token = gfd.slab_index();
                        if let Some(conn) = self.connections.get_mut(token) {
                            if conn.gfd != gfd {
                                continue;
                            }
                            let _ = conn.write(&data);
                        } else {
                            continue;
                        }

                        self.write_socket(token)?;
                        self.reregister(token)?;
                    }
                    Command::Register(socket, local_addr, peer_addr) => {
                        self.register(socket, local_addr, peer_addr)?
                    }
                    Command::Close(key) => self.close_connection(key, true)?,
                    Command::Read(key) => {
                        if self.read_socket(key)? {
                            let _ = self.inner_sender.send(Command::Read(key));
                        }
                    }
                    Command::Write(key) => {
                        if self.write_socket(key)? {
                            let _ = self.inner_sender.send(Command::Write(key));
                        }
                    }
                    Command::Wake() => todo!(),
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        Ok(())
    }

    fn accept_loop(&mut self) -> io::Result<()> {
        if self.listener.is_none() {
            return Ok(());
        }

        loop {
            let listener = self.listener.as_mut().unwrap();
            match listener.accept() {
                Ok((socket, peer_addr)) => {
                    let local_addr = listener.local_addr()?;
                    self.register(socket, local_addr, peer_addr)?;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    eprintln!("Accept error: {}", e);
                    break;
                }
            }
        }
        Ok(())
    }

    fn process_io(&mut self, token: Token, event: &Event) -> io::Result<()> {
        let key = token.into();

        if (event.is_error() || event.is_read_closed())
            && (!event.is_readable() && !event.is_writable())
        {
            self.close_connection(key, false)?;
            return Ok(());
        }

        if event.is_error() || event.is_writable() {
            self.write_socket(key)?;
        }

        if event.is_error() || event.is_readable() {
            self.read_socket(key)?;
        }

        Ok(())
    }

    fn register(
        &mut self,
        mut socket: Socket,
        local_addr: NetworkAddress,
        peer_addr: NetworkAddress,
    ) -> io::Result<()> {
        if let Err(e) = socket.set_nodelay(true) {
            eprintln!("set_nodelay failed: {}", e);
        }

        let entry = self.connections.vacant_entry();
        let token = Token(entry.key());
        let interests = Interest::READABLE;

        self.poll
            .registry()
            .register(&mut socket, token, interests)?;

        let raw_ptr = self.buffer.as_mut() as *mut IOBuffer;
        let ptr = NonNull::new(raw_ptr).expect("create ptr error");
        let gfd = Gfd::new(socket.fd(), self.loop_id, token.0);
        let mut conn = Connection::new(
            gfd,
            socket,
            self.options.clone(),
            local_addr,
            peer_addr,
            ptr,
        );
        match self.handler.on_open(&mut conn) {
            Action::Close => self.close_connection(token.0, true)?,
            Action::None => {
                entry.insert(conn);
                self.conn_count.fetch_add(1, Ordering::Relaxed);
            }
            Action::Publish(_) => {}
        }
        Ok(())
    }

    fn close_connection(&mut self, key: usize, graceful: bool) -> io::Result<()> {
        if !self.connections.contains(key) {
            return Ok(());
        }
        let conn = self.connections.get_mut(key).unwrap();

        if graceful {
            let _ = conn.flush();
        } else {
            conn.out_buf.clear();
        }

        self.handler.on_close(conn);
        let _ = self.poll.registry().deregister(&mut conn.socket);
        self.connections.remove(key);

        self.conn_count.fetch_sub(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_socket(&mut self, key: usize) -> io::Result<bool> {
        let status;
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(false),
        };

        if conn.closed {
            return Ok(false);
        }

        let mut bytes_read = 0;
        let max_batch_size = self.options.max_batch_size;

        loop {
            if bytes_read >= max_batch_size {
                status = IoStatus::Yield;
                break;
            }

            match conn.socket.read(&mut self.buffer) {
                Ok(0) => {
                    status = IoStatus::Closed(true);
                    break;
                }
                Ok(n) => {
                    bytes_read += n;
                    self.buffer.read(n);
                    match self.handler.on_traffic(conn, &mut self.cache) {
                        Action::None => {}
                        Action::Close => {
                            status = IoStatus::Closed(true);
                            break;
                        }
                        Action::Publish(job) => {
                            let req = Command::JobReq(conn.gfd.clone(), job);
                            let _ = self.job_sender.send(req); // todo handler error
                        }
                    }

                    if self.buffer.remaining() > 0 {
                        let buffer = self.buffer.remaining_bytes();
                        conn.in_buf.extend_from_slice(buffer);
                    }
                    self.cache.clear();
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    status = IoStatus::Completed;
                    break;
                }
                Err(_) => {
                    status = IoStatus::Closed(false);
                    break;
                }
            }
        }

        match status {
            IoStatus::Closed(graceful) => {
                self.close_connection(key, graceful)?;
                Ok(false)
            }
            IoStatus::Yield => Ok(true),
            IoStatus::Completed => Ok(false),
        }
    }

    fn write_socket(&mut self, key: usize) -> io::Result<bool> {
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(false),
        };

        let mut bytes_written = 0;
        let max_batch_size = self.options.max_batch_size;

        let mut status = IoStatus::Completed;
        while !conn.out_buf.is_empty() {
            if bytes_written >= max_batch_size {
                status = IoStatus::Yield;
                break;
            }

            match conn.socket.write(&conn.out_buf) {
                Ok(0) => {
                    status = IoStatus::Closed(false);
                    break;
                }
                Ok(n) => {
                    conn.out_buf.advance(n);
                    bytes_written += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    status = IoStatus::Completed;
                    break;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    status = IoStatus::Closed(false);
                    break;
                }
            }
        }

        match status {
            IoStatus::Closed(graceful) => {
                self.close_connection(key, graceful)?;
                Ok(false)
            }
            IoStatus::Yield => Ok(true),
            IoStatus::Completed => Ok(false),
        }
    }
}
