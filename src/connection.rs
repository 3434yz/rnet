use bytes::BytesMut;
use mio::net::TcpStream;
use std::net::SocketAddr;

pub struct Connection {
    pub stream: TcpStream,
    pub addr: SocketAddr,
    // 读缓冲区：从 Kernel 读入数据，等待用户解析
    pub in_buf: BytesMut,
    // 写缓冲区：用户写入数据，等待引擎 flush 到 Kernel
    pub out_buf: BytesMut,
    // 标记连接是否已关闭
    pub closed: bool,
}

impl Connection {
    pub fn new(stream: TcpStream, addr: SocketAddr) -> Self {
        Self {
            stream,
            addr,
            // 预分配内存，避免频繁扩容
            in_buf: BytesMut::with_capacity(4096),
            out_buf: BytesMut::with_capacity(4096),
            closed: false,
        }
    }
}
