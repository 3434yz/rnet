use bytes::BytesMut;
use mio::net::TcpStream;
use std::net::SocketAddr;

// TODO 增加 GFD
pub struct Connection {
    pub stream: TcpStream,
    pub local_addr: SocketAddr,
    pub peer_addr: SocketAddr,
    pub in_buf: BytesMut,
    pub out_buf: BytesMut,
    pub closed: bool,
}

impl Connection {
    pub fn new(stream: TcpStream, local_addr: SocketAddr, peer_addr: SocketAddr) -> Self {
        Self {
            stream,
            local_addr,
            peer_addr,
            in_buf: BytesMut::with_capacity(4096),
            out_buf: BytesMut::with_capacity(4096),
            closed: false,
        }
    }
}
