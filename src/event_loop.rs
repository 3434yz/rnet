use crate::command::{Command, Prioity};
use crate::connection::Connection;
use crate::gfd::Gfd;
use crate::handler::{Action, EventHandler};
use crate::listener::Listener;
use crate::options::Options;
use crate::poller::{Poller, Waker};
use crate::socket::Socket;
use crate::socket_addr::NetworkAddress;

use bytes::{Buf, BytesMut};
use crossbeam::channel::{Receiver, Sender, TryRecvError};
use mio::event::Event;
use mio::{Events, Token};
use slab::Slab;

use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const EVENTS_CAP: usize = 1024;
const MAX_POLL_EVENT_CAP: usize = 1024;
const MAX_ASYNC_TASK_ONE_TIME: usize = 256;

const IO_MAX: usize = 1024;

thread_local! {
    static IO_SLICES: RefCell<Vec<io::IoSlice<'static>>> = RefCell::new(Vec::with_capacity(IO_MAX));
}

enum IoStatus {
    Completed,
    Yield,
    Closed(bool),
    Shutdown,
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
    io_buffer: BytesMut,
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
        // let buffer = Some(IOBuffer::new(options.read_buffer_cap));
        let io_buffer = BytesMut::with_capacity(options.read_buffer_cap);
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
            io_buffer,
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
            let is_empty = conn.out_buffer.as_ref().map_or(true, |b| b.is_empty());
            if is_empty {
                self.poll.disable_write(&mut conn.socket, Token(key))?;
            } else {
                self.poll.enable_write(&mut conn.socket, Token(key))?;
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
                if token == crate::poller::WAKE_TOKEN {
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
                    let Some(conn) = self.connections.get_mut(token) else {
                        continue;
                    };
                    if conn.gfd != gfd {
                        continue;
                    }

                    if conn.write(&data).is_err() {
                        self.close_connection(token, false)?;
                        continue;
                    }

                    self.reregister(token)?;
                }
                Command::Register(socket, local_addr, peer_addr) => {
                    self.register(socket, local_addr, peer_addr)?
                }
                Command::Close(key) => self.close_connection(key, true)?,
                Command::Read(key) => {
                    self.read_socket(key)?;
                }
                Command::Write(key) => {
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
        self.reregister(key)?;

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

        let gfd = Gfd::new(socket.fd(), self.loop_id, token.0);
        let mut conn = Connection::new(
            gfd,
            socket,
            self.options.clone(),
            local_addr,
            peer_addr,
            self.handle.clone(),
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
            conn.out_buffer = None;
        }

        self.handler.on_close(conn);
        self.poll.deregister(&mut conn.socket)?;
        self.connections.remove(key);

        self.handle.conn_count.fetch_sub(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_socket(&mut self, key: usize) -> io::Result<()> {
        let status = self.process_read(key)?;

        match status {
            IoStatus::Closed(graceful) => {
                self.close_connection(key, graceful)?;
            }
            IoStatus::Yield => {
                let _ = self.trigger(Prioity::High, Command::Read(key));
            }
            IoStatus::Completed => {}
            IoStatus::Shutdown => {
                self.shutdown();
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "EventLoop Shutdown",
                ));
            }
        }
        Ok(())
    }

    fn process_read(&mut self, key: usize) -> io::Result<IoStatus> {
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(IoStatus::Completed),
        };

        if conn.closed {
            return Ok(IoStatus::Completed);
        }

        let mut bytes_read = 0;
        let max_batch_size = self.options.max_batch_size;
        let read_buffer_cap = self.options.read_buffer_cap;

        loop {
            if bytes_read >= max_batch_size {
                return Ok(IoStatus::Yield);
            }

            if self.io_buffer.capacity() < read_buffer_cap {
                self.io_buffer.reserve(read_buffer_cap);
            }
            if self.io_buffer.len() < read_buffer_cap {
                unsafe { self.io_buffer.set_len(read_buffer_cap) };
            }

            match conn.socket.read(&mut self.io_buffer) {
                Ok(0) => return Ok(IoStatus::Closed(true)),
                Ok(n) => {
                    bytes_read += n;
                    let io_buffer = self.io_buffer.split_to(n);
                    conn.io_buffer = Some(io_buffer);
                    match self.handler.on_traffic(conn) {
                        Action::None => {}
                        Action::Close => return Ok(IoStatus::Closed(true)),
                        Action::Shutdown => return Ok(IoStatus::Shutdown),
                    }

                    if let Some(io_buffer) = conn.io_buffer.take() {
                        if !io_buffer.is_empty() {
                            if let Some(in_buffer) = conn.in_buffer.as_mut() {
                                in_buffer.extend_from_slice(&io_buffer);
                            } else {
                                conn.in_buffer = Some(io_buffer);
                            }
                        }
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(IoStatus::Completed);
                }
                Err(_) => return Ok(IoStatus::Closed(false)),
            }
        }
    }

    fn write_socket(&mut self, key: usize) -> io::Result<()> {
        match self.process_write(key)? {
            IoStatus::Closed(graceful) => {
                self.close_connection(key, graceful)?;
            }
            IoStatus::Yield => {
                let _ = self.trigger(Prioity::High, Command::Write(key));
            }
            IoStatus::Completed => {
                self.reregister(key)?;
            }
            IoStatus::Shutdown => {
                let _ = self.trigger(Prioity::High, Command::Shutdown);
            }
        }
        Ok(())
    }

    fn process_write(&mut self, key: usize) -> io::Result<IoStatus> {
        let max_batch_size = self.options.max_batch_size;
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(IoStatus::Completed),
        };

        let out_buffer = match &mut conn.out_buffer {
            Some(buf) if !buf.is_empty() => buf,
            _ => return Ok(IoStatus::Completed),
        };

        let socket = &mut conn.socket;
        let mut bytes_written = 0;
        IO_SLICES.with(|cells| {
            let mut io_slices = cells.borrow_mut();
            while !out_buffer.is_empty() {
                if bytes_written >= max_batch_size {
                    return Ok(IoStatus::Yield);
                }

                io_slices.clear();
                for buf in out_buffer.iter().take(IO_MAX) {
                    let slice = io::IoSlice::new(buf);
                    let static_slice = unsafe {
                        std::mem::transmute::<io::IoSlice<'_>, io::IoSlice<'static>>(slice)
                    };
                    io_slices.push(static_slice);
                }

                let res = socket.write_vectored(&io_slices);
                io_slices.clear();

                match res {
                    Ok(0) => return Ok(IoStatus::Closed(false)),
                    Ok(n) => {
                        bytes_written += n;
                        let mut written = n;
                        while written > 0 {
                            if let Some(front) = out_buffer.front_mut() {
                                if front.len() <= written {
                                    written -= front.len();
                                    out_buffer.pop_front();
                                } else {
                                    front.advance(written);
                                    written = 0;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(IoStatus::Completed);
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return Ok(IoStatus::Closed(false)),
                }
            }
            Ok(IoStatus::Completed)
        })
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
