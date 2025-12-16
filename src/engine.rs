use crate::codec::Codec;
use crate::connection::Connection;
use crate::handler::{Action, Context, EventHandler};
use bytes::Buf;
use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token};
use slab::Slab;
use std::io::{self, Read, Write};

const SERVER_TOKEN: Token = Token(usize::MAX);
const SCRATCH_BUF_SIZE: usize = 64 * 1024;
const EVENTS_CAP: usize = 1024;

pub struct Engine<H, C>
where
    H: EventHandler,
    C: Codec,
{
    poll: Poll,
    listener: TcpListener,
    connections: Slab<Connection>,
    handler: H,
    codec: C,
    scratch_buf: Box<[u8; SCRATCH_BUF_SIZE]>,
}

impl<H, C> Engine<H, C>
where
    H: EventHandler<Message = C::Message>, // 约束：Handler 处理的消息必须匹配 Codec
    C: Codec + Clone, // Codec 需要 Clone 给每个 Worker (或者 Engine 本身就是每个 Worker 一个)
{
    pub fn new(mut listener: TcpListener, handler: H, codec: C) -> io::Result<Self> {
        let poll = Poll::new()?;

        poll.registry()
            .register(&mut listener, SERVER_TOKEN, Interest::READABLE)?;

        Ok(Self {
            poll,
            listener,
            connections: Slab::with_capacity(4096),
            handler,
            codec,
            scratch_buf: Box::new([0u8; SCRATCH_BUF_SIZE]),
        })
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
                }
            }
        }
    }

    #[inline]
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

    #[inline]
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

            // 2. Decode Loop (关键新增)
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
                            match self.handler.on_traffic(&mut ctx, msg) {
                                Action::Close => {
                                    closed = true;
                                    break;
                                }
                                Action::None => {
                                    // 可以在这里 encode 回包，但通常由 Handler 往 ctx.out_buf 写
                                    // 如果 Handler 想回包，它应该自己处理 encode，或者我们提供 helper
                                    // 目前模型：Handler 拿到 msg，自己决定往 out_buf 写什么 (bytes)
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
}
