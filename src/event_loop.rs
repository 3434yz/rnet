use crate::command::{Command, Prioity};
use crate::connection::Connection;
use crate::gfd::Gfd;
use crate::handler::{Action, EventHandler};
use crate::io_buffer::IOBuffer;
use crate::listener::Listener;
use crate::options::Options;
use crate::poller::{Poller, WAKE_TOKEN, Waker};
use crate::socket::Socket;
use crate::socket_addr::NetworkAddress;

use bytes::{Buf, BytesMut};
use crossbeam::channel::{Receiver, Sender, TryRecvError};
use mio::event::Event;
use mio::{Events, Token};
use slab::Slab;

use std::io::{self, Read, Write};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const EVENTS_CAP: usize = 1024;
const MAX_POLL_EVENT_CAP: usize = 1024;
const MAX_ASYNC_TASK_ONE_TIME: usize = 256;

enum IoStatus {
    Completed,
    Yield,
    Closed(bool),
}

#[derive(Clone, Debug)]
pub struct EventLoopHandle {
    pub idx: usize,
    pub urgent_sender: Sender<Command>,
    pub common_sender: Sender<Command>,
    pub waker: Arc<Waker>,
    pub conn_count: Arc<AtomicUsize>,
}

impl EventLoopHandle {
    pub fn new(
        idx: usize,
        urgent_sender: Sender<Command>,
        common_sender: Sender<Command>,
        waker: Arc<Waker>,
        conn_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            idx,
            urgent_sender,
            common_sender,
            waker,
            conn_count,
        }
    }

    pub(crate) fn shutdown(&self) {
        let command = Command::Shutdown;
        let priority = Command::Shutdown.priority();
        let _ = self.trigger(priority, command);
    }

    pub(crate) fn connection_count(&self) -> usize {
        self.conn_count.load(Ordering::Relaxed)
    }

    pub(crate) fn trigger(&self, priority: Prioity, command: Command) -> io::Result<bool> {
        let res;
        if priority > Prioity::High && self.urgent_sender.len() >= MAX_POLL_EVENT_CAP {
            res = self.common_sender.send(command);
        } else {
            res = self.urgent_sender.send(command);
        }

        if res.is_err() {
            return Ok(false);
        }

        if self.waker.wake().is_err() {
            return Ok(false);
        } else {
            return Ok(true);
        }
    }
}

pub(crate) struct EventLoop<H>
where
    H: EventHandler,
{
    pub loop_id: u8,
    poll: Poller,
    listeners: Option<Vec<Listener>>,
    options: Arc<Options>,
    connections: Slab<Connection>,
    handler: Arc<H>,
    buffer: Box<IOBuffer>,
    cache: BytesMut,
    urgent_receiver: Receiver<Command>,
    common_receiver: Receiver<Command>,
    handle: Arc<EventLoopHandle>,
}

impl<H> EventLoop<H>
where
    H: EventHandler,
{
    pub(crate) fn new(
        loop_id: u8,
        options: Arc<Options>,
        handler: Arc<H>,
        urgent_sender: Sender<Command>,
        urgent_receiver: Receiver<Command>,
        common_sender: Sender<Command>,
        common_receiver: Receiver<Command>,
        conn_count: Arc<AtomicUsize>,
    ) -> io::Result<Self> {
        let poll = Poller::new()?;
        let waker = Arc::new(Waker::new(poll.registry())?);
        let buffer = Box::new(IOBuffer::new(options.read_buffer_cap));
        let cache = BytesMut::with_capacity(options.read_buffer_cap);
        let connections = Slab::with_capacity(4096);

        let handle = EventLoopHandle::new(
            loop_id as usize,
            urgent_sender,
            common_sender,
            waker,
            conn_count,
        );
        let handle = Arc::new(handle);

        Ok(Self {
            loop_id,
            poll,
            listeners: None,
            connections,
            handler,
            buffer,
            cache,
            urgent_receiver,
            common_receiver,
            handle,
            options,
        })
    }

    pub(crate) fn listener(mut self, listeners: Vec<Listener>) -> Self {
        self.listeners = Some(listeners);
        self
    }

    pub(crate) fn build(mut self) -> io::Result<Self> {
        if let Some(listeners) = &mut self.listeners {
            for (i, listener) in listeners.iter_mut().enumerate() {
                self.poll
                    .register(listener, crate::poller::listener_token(i))?;
            }
        }
        Ok(self)
    }

    fn reregister(&mut self, key: usize) -> io::Result<()> {
        if let Some(conn) = self.connections.get_mut(key) {
            if !conn.out_buf.is_empty() {
                self.poll.enable_write(&mut conn.socket, Token(key))?;
            } else {
                self.poll.disable_write(&mut conn.socket, Token(key))?;
            }
        }
        Ok(())
    }

    pub(crate) fn handle(&self) -> Arc<EventLoopHandle> {
        self.handle.clone()
    }

    pub(crate) fn run(&mut self) -> io::Result<()> {
        match self.run_loop() {
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => Ok(()),
            other => other,
        }
    }

    fn run_loop(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(EVENTS_CAP);
        loop {
            let has_pending = !self.urgent_receiver.is_empty() || !self.common_receiver.is_empty();
            let timeout = if has_pending {
                Some(Duration::ZERO)
            } else {
                None
            };

            if let Err(e) = self.poll.poll(&mut events, timeout) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }

            if has_pending {
                self.process_command()?;
            }

            for event in events.iter() {
                let token = event.token();
                if token == WAKE_TOKEN {
                    self.handle.waker.reset();
                    self.process_command()?;
                } else if let Some(idx) = crate::poller::is_listener_token(token) {
                    self.accept_process(idx)?;
                } else {
                    self.process_io(token, &event)?;
                }
            }
        }
    }

    fn process_command(&mut self) -> io::Result<()> {
        for _ in 0..MAX_ASYNC_TASK_ONE_TIME {
            let command = match self.urgent_receiver.try_recv() {
                Ok(c) => c,
                Err(TryRecvError::Empty) => match self.common_receiver.try_recv() {
                    Ok(c) => c,
                    Err(_) => break,
                },
                Err(TryRecvError::Disconnected) => break,
            };

            match command {
                Command::None => {}
                Command::AsyncWrite(gfd, data) => {
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
                Command::IORead(key) => {
                    self.read_socket(key)?;
                }
                Command::IOWrite(key) => {
                    self.write_socket(key)?;
                }
                Command::Wake => self.handle.waker.wake()?,
                Command::Shutdown => {
                    self.shutdown();
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "EventLoop Shutdown",
                    ));
                }
            }
        }
        Ok(())
    }

    fn accept_process(&mut self, idx: usize) -> io::Result<()> {
        let Some(listeners) = self.listeners.take() else {
            return Ok(());
        };

        if let Some(listener) = listeners.get(idx) {
            loop {
                match listener.accept() {
                    Ok((socket, peer_addr)) => {
                        let local_addr = match listener.local_addr() {
                            Ok(addr) => addr,
                            Err(e) => {
                                self.listeners = Some(listeners);
                                return Err(e);
                            }
                        };
                        if let Err(e) = self.register(socket, local_addr, peer_addr) {
                            self.listeners = Some(listeners);
                            return Err(e);
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        eprintln!("Accept error: {}", e);
                        break;
                    }
                }
            }
        }
        self.listeners = Some(listeners);
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
        use crate::options::TcpSocketOpt;

        if matches!(self.options.tcp_no_delay, TcpSocketOpt::NoDelay) {
            if let Err(e) = socket.set_nodelay(true) {
                eprintln!("set_nodelay failed: {}", e);
            }
        }

        let entry = self.connections.vacant_entry();
        let token = Token(entry.key());

        self.poll.register(&mut socket, token)?;

        let raw_ptr = self.buffer.as_mut() as *mut IOBuffer;
        let ptr = NonNull::new(raw_ptr).expect("create ptr error");
        let gfd = Gfd::new(socket.fd(), self.loop_id, token.0);
        let mut conn = Connection::new(
            gfd,
            socket,
            self.options.clone(),
            local_addr,
            peer_addr,
            self.handle.clone(),
            ptr,
        );
        match self.handler.on_open(&mut conn) {
            Action::Close => self.close_connection(token.0, true)?,
            Action::None => {
                entry.insert(conn);
                self.handle.conn_count.fetch_add(1, Ordering::Relaxed);
            }
            Action::Shutdown => {
                self.shutdown();
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "EventLoop Shutdown",
                ));
            }
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
        self.poll.deregister(&mut conn.socket)?;
        self.connections.remove(key);

        self.handle.conn_count.fetch_sub(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_socket(&mut self, key: usize) -> io::Result<()> {
        let status;
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(()),
        };

        if conn.closed {
            return Ok(());
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
                        Action::Shutdown => {
                            self.shutdown();
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "EventLoop Shutdown",
                            ));
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
            }
            IoStatus::Yield => {
                let _ = self.trigger(Prioity::High, Command::IORead(key));
            }
            IoStatus::Completed => {}
        }
        Ok(())
    }

    fn write_socket(&mut self, key: usize) -> io::Result<()> {
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(()),
        };

        let mut bytes_written = 0;
        let max_batch_size = self.options.max_batch_size;

        let mut status = IoStatus::Completed;
        while !conn.out_buf.is_empty() {
            if bytes_written >= max_batch_size {
                status = IoStatus::Yield;
                break;
            }

            let total_len = conn.out_buf.len();
            let remaining = max_batch_size - bytes_written;
            let actual_len = std::cmp::min(remaining, total_len);
            let data = &conn.out_buf[0..actual_len];

            match conn.socket.write(data) {
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
            }
            IoStatus::Yield => {
                let _ = self.trigger(Prioity::High, Command::IOWrite(key));
            }
            IoStatus::Completed => {}
        }
        Ok(())
    }

    fn trigger(&self, priority: Prioity, command: Command) -> io::Result<bool> {
        self.handle.trigger(priority, command)
    }

    fn shutdown(&mut self) {
        if let Some(mut listeners) = self.listeners.take() {
            for listener in &mut listeners {
                let _ = self.poll.deregister(listener);
            }
        }

        let keys: Vec<usize> = self.connections.iter().map(|(k, _)| k).collect();
        for key in keys {
            let _ = self.close_connection(key, true);
        }
    }
}
