use crate::command::{Request, Response};
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

use std::collections::VecDeque;
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
    Closed,
}

#[derive(Debug)]
pub struct ConnectionInitializer {
    pub socket: Socket,
    pub peer_addr: NetworkAddress,
    pub local_addr: NetworkAddress,
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
pub struct EventLoopHandle {
    pub idx: usize,
    pub sender: Sender<ConnectionInitializer>,
    pub waker: Arc<EventLoopWaker>,
    pub conn_count: Arc<AtomicUsize>,
}

impl EventLoopHandle {
    pub fn new(
        idx: usize,
        sender: Sender<ConnectionInitializer>,
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
    request_sender: Sender<Request<H::Job>>,
    response_receiver: Receiver<Response>,
    conn_receiver: Option<Receiver<ConnectionInitializer>>,
    conn_count: Option<Arc<AtomicUsize>>,
    resume_queue: VecDeque<usize>,
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
        conn_receiver: Option<Receiver<ConnectionInitializer>>,
        request_sender: Sender<Request<H::Job>>,
        response_receiver: Receiver<Response>,
        conn_count: Option<Arc<AtomicUsize>>,
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
        let resume_queue = VecDeque::with_capacity(1024);

        Ok(Self {
            loop_id: idx,
            poll,
            listener,
            connections,
            handler,
            buffer,
            cache,
            request_sender,
            response_receiver,
            conn_receiver,
            waker,
            conn_count,
            options,
            resume_queue,
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
            let timeout = if self.resume_queue.is_empty() {
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
                        self.process_responses()?;
                        self.process_new_connections()?;
                    }
                    token => self.process_io(token, &event)?,
                }
            }

            let pending_count = self.resume_queue.len();
            for _ in 0..pending_count {
                if let Some(token_idx) = self.resume_queue.pop_front() {
                    if self.process_socket_resume(token_idx)? {
                        self.close_connection(token_idx)?;
                    }
                }
            }
        }
    }

    fn process_socket_resume(&mut self, key: usize) -> io::Result<bool> {
        let mut yield_flag = false;
        match self.read_socket(key)? {
            IoStatus::Closed => return Ok(true),
            IoStatus::Yield => yield_flag = true,
            IoStatus::Completed => {}
        }

        match self.write_socket(key)? {
            IoStatus::Closed => return Ok(true),
            IoStatus::Yield => yield_flag = true,
            IoStatus::Completed => {}
        }

        if yield_flag {
            self.resume_queue.push_back(key);
        } else {
            self.reregister(key)?;
        }
        Ok(false)
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

    fn process_new_connections(&mut self) -> io::Result<()> {
        let receiver = match &self.conn_receiver {
            Some(r) => r.clone(),
            None => return Ok(()),
        };

        loop {
            let init = match receiver.try_recv() {
                Ok(s) => s,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            };

            self.register(init.socket, init.local_addr, init.peer_addr)?;
        }
        Ok(())
    }

    fn process_io(&mut self, token: Token, event: &Event) -> io::Result<()> {
        let key = token.into();
        let mut yield_flag = false;

        if event.is_writable() {
            match self.write_socket(key)? {
                IoStatus::Closed => {
                    self.close_connection(key)?;
                    return Ok(());
                }
                IoStatus::Yield => {
                    yield_flag = true;
                }
                IoStatus::Completed => {}
            }
        }

        if event.is_readable() {
            match self.read_socket(key)? {
                IoStatus::Closed => {
                    self.close_connection(key)?;
                    return Ok(());
                }
                IoStatus::Yield => {
                    yield_flag = true;
                }
                IoStatus::Completed => {}
            }
        }

        if yield_flag {
            self.resume_queue.push_back(key);
        } else {
            self.reregister(key)?;
        }

        Ok(())
    }

    fn process_responses(&mut self) -> io::Result<()> {
        while let Ok(response) = self.response_receiver.try_recv() {
            let gfd = response.gfd;
            let loop_idx = gfd.event_loop_index();
            if loop_idx != self.loop_id as usize {
                eprintln!("Error: Received response for wrong engine {}", loop_idx);
                continue;
            }
            let token = gfd.slab_index();
            if let Some(conn) = self.connections.get_mut(token) {
                if conn.gfd != gfd {
                    // Connection mismatch (ABA problem), ignore stale response
                    continue;
                }
                let _ = conn.write(&response.data);
            } else {
                continue;
            }

            let status = self.write_socket(token)?;
            if matches!(status, IoStatus::Closed) {
                self.close_connection(token)?;
            } else {
                self.reregister(token)?;
            }
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
            Action::Close => {
                // Drop conn
            }
            Action::None => {
                entry.insert(conn);
                if let Some(cnt) = &self.conn_count {
                    cnt.fetch_add(1, Ordering::Relaxed);
                }
            }
            Action::Publish(_) => {}
        }
        Ok(())
    }

    fn close_connection(&mut self, key: usize) -> io::Result<()> {
        if !self.connections.contains(key) {
            return Ok(());
        }
        let conn = self.connections.get_mut(key).unwrap();
        self.handler.on_close(conn);
        let _ = self.poll.registry().deregister(&mut conn.socket);
        self.connections.remove(key);

        if let Some(cnt) = &self.conn_count {
            cnt.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn read_socket(&mut self, key: usize) -> io::Result<IoStatus> {
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(IoStatus::Completed),
        };

        if conn.closed {
            return Ok(IoStatus::Completed);
        }

        let mut bytes_read = 0;
        let max_batch_size = self.options.max_batch_size;

        loop {
            if bytes_read >= max_batch_size {
                return Ok(IoStatus::Yield);
            }

            match conn.socket.read(&mut self.buffer) {
                Ok(0) => return Ok(IoStatus::Completed),
                Ok(n) => {
                    bytes_read += n;
                    self.buffer.read(n);
                    match self.handler.on_traffic(conn, &mut self.cache) {
                        Action::None => {}
                        Action::Close => return Ok(IoStatus::Closed),
                        Action::Publish(job) => {
                            let req = Request {
                                gfd: conn.gfd.clone(),
                                job,
                            };
                            let _ = self.request_sender.send(req);
                        }
                    }

                    if self.buffer.remaining() > 0 {
                        let buffer = self.buffer.remaining_bytes();
                        conn.in_buf.extend_from_slice(buffer);
                    }
                    self.cache.clear();
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(IoStatus::Completed);
                }
                Err(_) => return Ok(IoStatus::Closed),
            }
        }
    }

    fn write_socket(&mut self, key: usize) -> io::Result<IoStatus> {
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(IoStatus::Completed),
        };

        let mut bytes_written = 0;
        let quota = self.options.max_batch_size;

        while !conn.out_buf.is_empty() {
            if bytes_written >= quota {
                return Ok(IoStatus::Yield);
            }

            match conn.socket.write(&conn.out_buf) {
                Ok(n) => {
                    conn.out_buf.advance(n);
                    bytes_written += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(IoStatus::Completed);
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Ok(IoStatus::Closed),
            }
        }
        Ok(IoStatus::Completed)
    }
}
