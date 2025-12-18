use crate::codec::Codec;
use crate::connection::Connection;
use crate::handler::{Action, Context, EventHandler};

use bytes::Buf;
use crossbeam::channel::{Receiver, Sender};
use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token, Waker};
use slab::Slab;

use crate::command::{pack_session_id,unpack_session_id, Request, Response};
use mio::event::Event;
use std::io::{self, Read, Write};
use std::sync::Arc;

const SERVER_TOKEN: Token = Token(usize::MAX);
const WAKE_TOKEN: Token = Token(usize::MAX - 1);
const SCRATCH_BUF_SIZE: usize = 64 * 1024;
const EVENTS_CAP: usize = 1024;

pub struct Engine<H, C, J>
where
    H: EventHandler,
    C: Codec,
{
    engine_id: usize,
    poll: Poll,
    listener: TcpListener,
    connections: Slab<Connection>,
    handler: H,
    codec: C,
    scratch_buf: Box<[u8; SCRATCH_BUF_SIZE]>,
    request_sender: Sender<Request<J>>,
    response_receiver: Receiver<Response>,
    waker: Arc<Waker>,
}

impl<H, C, J> Engine<H, C, J>
where
    H: EventHandler<Message = C::Message, Job = J>,
    C: Codec + Clone,
{
    pub fn new(
        engine_id: usize,
        mut listener: TcpListener,
        handler: H,
        codec: C,
        request_sender: Sender<Request<J>>,
        response_receiver: Receiver<Response>,
    ) -> io::Result<Self> {
        let poll = Poll::new()?;

        poll.registry()
            .register(&mut listener, SERVER_TOKEN, Interest::READABLE)?;

        let waker = Waker::new(poll.registry(), WAKE_TOKEN)?;
        let waker = Arc::new(waker);

        Ok(Self {
            engine_id,
            poll,
            listener,
            connections: Slab::with_capacity(4096),
            handler,
            codec,
            scratch_buf: Box::new([0u8; SCRATCH_BUF_SIZE]),
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
                    token => self.conn_event(token)?,
                    WAKE_TOKEN=>self.process_responses()?,
                }
            }
        }
    }

    fn accept_loop(&mut self) -> io::Result<()> {
        loop {
            match self.listener.accept() {
                Ok((mut stream, addr)) => {
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
                    let mut conn = Connection::new(stream, addr);

                    // 5. 触发 OnOpen 回调
                    let mut ctx = Context {
                        local_addr: conn.addr, // 简化处理，暂时用 remote 代替 local 用于展示
                        peer_addr: conn.addr,
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
                        Action::Publish(_)=>{}
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

    fn conn_event(&mut self, token: Token) -> io::Result<()> {
        let key = token.into();
        if let Some(conn) = self.connections.get_mut(key) {
            let mut closed = false;

            // 1. Read into in_buf
            loop {
                match conn.stream.read(&mut self.scratch_buf[..]) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => {
                        conn.in_buf.extend_from_slice(&self.scratch_buf[..n]);
                        if n < SCRATCH_BUF_SIZE {
                            break;
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }
            }

            if !closed {
                loop {
                    // 调用 Codec 解码
                    match self.codec.decode(&mut conn.in_buf) {
                        Ok(Some(msg)) => {
                            // 解码成功，调用 Handler
                            let mut ctx = Context {
                                local_addr: conn.addr,
                                peer_addr: conn.addr,
                                out_buf: &mut conn.out_buf,
                            };

                            // 此时 msg 已经是完整的包
                            match self.handler.on_message(&mut ctx, msg) {
                                Action::Close => {
                                    closed = true;
                                    break;
                                }
                                Action::None => {
                                    // 可以在这里 encode 回包，但通常由 Handler 往 ctx.out_buf 写
                                    // 如果 Handler 想回包，它应该自己处理 encode，或者我们提供 helper
                                    // 目前模型：Handler 拿到 msg，自己决定往 out_buf 写什么 (bytes)
                                }
                                Action::Publish(job) => {
                                    let req = Request {
                                        session_id: pack_session_id(self.engine_id, token.0),
                                        job,
                                    };
                                    let _ = self.request_sender.send(req);
                                }
                            }
                        }
                        Ok(None) => {
                            // 数据不够，等待更多数据
                            break;
                        }
                        Err(e) => {
                            eprintln!("Codec Error: {}", e);
                            closed = true;
                            break;
                        }
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
                    local_addr: conn.addr,
                    peer_addr: conn.addr,
                    out_buf: &mut conn.out_buf,
                };
                self.handler.on_close(&mut ctx);
                let _ = self.poll.registry().deregister(&mut conn.stream);
                self.connections.remove(key);
            }
        }
        Ok(())
    }

    fn process_responses(&mut self)->io::Result<()> {
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
                    // 构造临时 Context 触发 on_close 回调
                    let mut ctx = Context {
                        local_addr: conn.addr,
                        peer_addr: conn.addr,
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
