use crate::connection::Connection;
use crate::handler::{Action, Context, EventHandler};

use bytes::{Buf, BufMut, BytesMut};
use crossbeam::channel::{Receiver, Sender};
use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token, Waker};
use slab::Slab;

use crate::command::{Request, Response, pack_session_id, unpack_session_id};
use mio::event::Event;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;

const SERVER_TOKEN: Token = Token(usize::MAX);
const WAKE_TOKEN: Token = Token(usize::MAX - 1);
const SCRATCH_BUF_SIZE: usize = 64 * 1024;
const EVENTS_CAP: usize = 1024;

pub struct Engine<H>
where
    H: EventHandler,
{
    engine_id: usize,
    poll: Poll,
    listener: TcpListener,
    local_addr: SocketAddr,
    connections: Slab<Connection>,
    handler: H,
    scratch_buf: [u8; SCRATCH_BUF_SIZE],
    request_sender: Sender<Request<H::Job>>,
    response_receiver: Receiver<Response>,
    waker: Arc<Waker>,
}

impl<H> Engine<H>
where
    H: EventHandler,
{
    pub fn new(
        engine_id: usize,
        mut listener: TcpListener,
        handler: H,
        request_sender: Sender<Request<H::Job>>,
        response_receiver: Receiver<Response>,
    ) -> io::Result<Self> {
        let poll = Poll::new()?;

        poll.registry()
            .register(&mut listener, SERVER_TOKEN, Interest::READABLE)?;

        let waker = Waker::new(poll.registry(), WAKE_TOKEN)?;
        let waker = Arc::new(waker);
        let local_addr = listener.local_addr()?;
        let scratch_buf = [0; SCRATCH_BUF_SIZE];

        Ok(Self {
            engine_id,
            poll,
            listener,
            local_addr,
            connections: Slab::with_capacity(4096),
            handler,
            scratch_buf,
            request_sender,
            response_receiver,
            waker,
        })
    }

    pub fn get_waker(&self) -> Arc<Waker> {
        self.waker.clone()
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(EVENTS_CAP);
        loop {
            if let Err(e) = self.poll.poll(&mut events, None) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }

            for event in events.iter() {
                match event.token() {
                    SERVER_TOKEN => self.accept_loop()?,
                    WAKE_TOKEN => self.process_responses()?,
                    token => self.conn_event(token, &event)?,
                }
            }
        }
    }

    fn accept_loop(&mut self) -> io::Result<()> {
        loop {
            match self.listener.accept() {
                Ok((mut stream, peer_addr)) => {
                    // 1. 设置 Socket 选项 (Nodelay 是必须的)
                    if let Err(e) = stream.set_nodelay(true) {
                        eprintln!("set_nodelay failed: {}", e);
                        continue;
                    }
                    // 注意：buffer size 应该在 listener 创建时设置（继承），
                    // 或者在这里显式设置 stream 的 buffer。
                    // 建议在 create_listener 中统一处理，这里只管 accept。

                    // 2. 注册到 Slab
                    let entry = self.connections.vacant_entry();
                    let token = Token(entry.key());

                    // 3. 注册到 Mio
                    self.poll
                        .registry()
                        .register(&mut stream, token, Interest::READABLE)?;

                    // 4. 创建 Connection 对象
                    let mut conn = Connection::new(stream, self.local_addr, peer_addr);

                    // 5. 触发 OnOpen 回调
                    let mut ctx = Context {
                        local_addr: conn.local_addr,
                        peer_addr: conn.peer_addr,
                        out_buf: &mut conn.out_buf,
                    };

                    match self.handler.on_open(&mut ctx) {
                        Action::Close => {
                            // 如果用户在 OnOpen 里要求关闭，就不 Insert 了
                            // Drop conn 会自动关闭 socket
                        }
                        Action::None => {
                            entry.insert(conn);
                        }
                        Action::Publish(_) => {}
                    }
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

    fn conn_event(&mut self, token: Token, event: &Event) -> io::Result<()> {
        let key = token.into();
        let conn = match self.connections.get_mut(key) {
            Some(c) => c,
            None => return Ok(()),
        };

        if conn.closed {
            return Ok(());
        }

        let mut closed = false;
        if event.is_readable() {
            loop {
                match conn.stream.read(&mut self.scratch_buf) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => {
                        conn.in_buf.extend_from_slice(&self.scratch_buf[..n]);
                        let mut ctx = Context {
                            local_addr: conn.local_addr,
                            peer_addr: conn.peer_addr,
                            out_buf: &mut conn.out_buf,
                        };

                        let action = self.handler.on_traffic(&mut ctx, &mut conn.in_buf);
                        match action {
                            Action::None => {} // 用户处理完了，或者只是读了一半等待更多数据
                            Action::Close => {
                                closed = true;
                            }
                            Action::Publish(job) => {
                                // 发送给 Worker
                                let req = Request {
                                    session_id: pack_session_id(self.engine_id, usize::from(token)),
                                    job,
                                };
                                let _ = self.request_sender.send(req);
                            }
                        }

                        // 如果读取没填满 scratch_buf，说明内核缓冲区空了，跳出读取循环
                        if n < self.scratch_buf.len() {
                            break;
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }

                if closed {
                    break
                }
            }
        }

        // 3. Write
        while !conn.out_buf.is_empty() {
            match conn.stream.write(&conn.out_buf) {
                Ok(n) => conn.out_buf.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }

        if closed {
            let mut ctx = Context {
                local_addr: conn.local_addr,
                peer_addr: conn.local_addr,
                out_buf: &mut conn.out_buf,
            };
            self.handler.on_close(&mut ctx);
            let _ = self.poll.registry().deregister(&mut conn.stream);
            self.connections.remove(key);
        }
        Ok(())
    }

    fn process_responses(&mut self) -> io::Result<()> {
        while let Ok(response) = self.response_receiver.try_recv() {
            let (engine_id, token_usize) = unpack_session_id(response.session_id);

            if engine_id != self.engine_id {
                eprintln!("Error: Received response for wrong engine {}", engine_id);
                continue;
            }

            if let Some(conn) = self.connections.get_mut(token_usize) {
                conn.out_buf.extend_from_slice(&response.data);

                let mut closed = false;
                while !conn.out_buf.is_empty() {
                    match conn.stream.write(&conn.out_buf) {
                        Ok(n) => {
                            conn.out_buf.advance(n);
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            eprintln!("Write error in process_responses: {}", e);
                            closed = true;
                            break;
                        }
                    }
                }

                if closed {
                    let mut ctx = Context {
                        local_addr: conn.local_addr,
                        peer_addr: conn.local_addr,
                        out_buf: &mut conn.out_buf,
                    };
                    self.handler.on_close(&mut ctx);
                    let _ = self.poll.registry().deregister(&mut conn.stream);
                    self.connections.remove(token_usize);
                }
            } else {
                // 这种情况可能发生：
                // Worker 在处理任务时，客户端断开了连接，Engine 已经移除了 Connection。
                // 此时 Worker 发回来的结果只能丢弃。
                // 这是一个正常的竞态条件，无需 Panic。
            }
        }
        Ok(())
    }
}
