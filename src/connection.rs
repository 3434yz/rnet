use crate::command::Command;
use crate::event_loop::EventLoopHandle;
use crate::gfd::Gfd;
use crate::io_buffer::{self, IOBuffer};
use crate::options::Options;
use crate::socket::Socket;
use crate::socket_addr::NetworkAddress;

use bytes::{Buf, Bytes, BytesMut};

use std::io::{Read, Write};
use std::ops::Deref;
use std::sync::Arc;

pub enum PeekData<'a> {
    Raw(&'a [u8]),
    BytesMut(BytesMut),
}

impl Deref for PeekData<'_> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            PeekData::Raw(data) => *data,
            PeekData::BytesMut(data) => &data[..],
        }
    }
}

pub struct Connection {
    pub(crate) gfd: Gfd,
    pub(crate) socket: Socket,
    pub(crate) local_addr: NetworkAddress,
    pub(crate) peer_addr: NetworkAddress,
    pub(crate) closed: bool,
    pub(crate) io_buffer: Option<BytesMut>,
    pub(crate) in_buffer: Option<BytesMut>,
    pub(crate) out_buf: BytesMut,
    pub(crate) handle: Arc<EventLoopHandle>,
}

impl Connection {
    pub fn new(
        gfd: Gfd,
        socket: Socket,
        options: Arc<Options>,
        local_addr: NetworkAddress,
        peer_addr: NetworkAddress,
        handle: Arc<EventLoopHandle>,
    ) -> Self {
        Self {
            gfd,
            socket,
            local_addr,
            peer_addr,
            closed: false,
            in_buffer: None,
            io_buffer: None,
            out_buf: BytesMut::with_capacity(options.write_buffer_cap),
            handle,
        }
    }

    pub fn gfd(&self) -> Gfd {
        self.gfd
    }

    pub fn local_addr(&self) -> &NetworkAddress {
        &self.local_addr
    }

    pub fn peer_addr(&self) -> &NetworkAddress {
        &self.peer_addr
    }

    pub(crate) fn in_buf_len(&self) -> usize {
        match self.in_buffer {
            Some(ref buf) => buf.len(),
            None => 0,
        }
    }

    pub(crate) fn io_buf_len(&self) -> usize {
        match self.io_buffer {
            Some(ref buf) => buf.remaining(),
            None => 0,
        }
    }

    pub fn next(&mut self, n: Option<usize>) -> Option<BytesMut> {
        let in_buffer_len = self.in_buf_len();
        let io_buffer_len = self.io_buf_len();
        let total_len = in_buffer_len + io_buffer_len;

        let actual_len = match n {
            Some(n) if n > total_len => return None,
            Some(n) => n,
            None => total_len,
        };

        if in_buffer_len == 0 {
            if let Some(io_buffer_mut) = self.io_buffer.as_mut() {
                let data = io_buffer_mut.split_to(actual_len);
                return Some(data);
            }
            return None;
        }

        let remaining = actual_len;
        let Some(in_buf_mut) = self.in_buffer.as_mut() else {
            return None;
        };

        if remaining <= in_buffer_len {
            let data = in_buf_mut.split_to(actual_len);
            return Some(data);
        }

        /*
            到这里代表获取in_buffer+io_buffer的数据
            ByestMut 在余量不足时可能原地扩容，所以
            这里选择直接把io_buffer中的数据拷贝到
            in_buffer中，发生拷贝的数据量在理想情况下
            只有 actual_len - in_buffer_len 。同时
            io_buffer在下一轮eventloop可以复用。
        */

        let io_buffer_mut = self.io_buffer.as_mut().unwrap();
        let copy_by_io = remaining - in_buffer_len;
        let mut in_buffer = self.in_buffer.take().unwrap();
        in_buffer.extend_from_slice(&io_buffer_mut[..copy_by_io]);
        io_buffer_mut.advance(copy_by_io);
        return Some(in_buffer);
    }

    pub fn peek(&self, n: Option<usize>) -> Option<PeekData<'_>> {
        let in_buffer_len = self.in_buf_len();
        let io_buffer_len = self.io_buf_len();
        let total_len = in_buffer_len + io_buffer_len;

        let actual_len = match n {
            Some(n) if n > total_len => return None,
            Some(n) => n,
            None => total_len,
        };

        if in_buffer_len == 0 {
            if let Some(io_buffer_mut) = self.io_buffer.as_ref() {
                return Some(PeekData::Raw(&io_buffer_mut[..actual_len]));
            }
            return None;
        }

        let remaining = actual_len;
        let Some(in_buf_mut) = self.in_buffer.as_ref() else {
            return None;
        };

        if remaining <= in_buffer_len {
            return Some(PeekData::Raw(&in_buf_mut[..actual_len]));
        }

        let copy_by_io = remaining - in_buffer_len;
        let io_buffer_mut = self.io_buffer.as_ref().unwrap();
        let mut data = BytesMut::with_capacity(actual_len);
        data.extend_from_slice(in_buf_mut);
        data.extend_from_slice(&io_buffer_mut[..copy_by_io]);
        return Some(PeekData::BytesMut(data));
    }

    pub fn discard(&mut self, n: Option<usize>) -> Option<usize> {
        let in_buffer_len = self.in_buf_len();
        let io_buffer_len = self.io_buf_len();
        let total_len = in_buffer_len + io_buffer_len;

        let actual_len = match n {
            Some(n) if n > total_len => return None,
            Some(n) => n,
            None => total_len,
        };

        if in_buffer_len == 0 {
            if let Some(io_buffer_mut) = self.io_buffer.as_mut() {
                io_buffer_mut.advance(actual_len);
                return Some(actual_len);
            }
            return None;
        }

        let remaining = actual_len;
        let in_buf_mut = self.in_buffer.as_mut().unwrap();

        if remaining <= in_buffer_len {
            in_buf_mut.advance(remaining);
            if in_buf_mut.is_empty() {
                self.in_buffer = None;
            }
            return Some(actual_len);
        }

        self.in_buffer = None;
        let discard_from_io = remaining - in_buffer_len;
        let io_buffer_mut = self.io_buffer.as_mut().unwrap();
        io_buffer_mut.advance(discard_from_io);
        Some(actual_len)
    }

    pub fn close(&mut self) {
        let cmd = Command::Close(self.gfd.slab_index());
        let _ = self.handle.trigger(cmd.priority(), cmd);
    }

    pub fn aysnc_write(&mut self, buf: Bytes) {
        let cmd = Command::AsyncWrite(self.gfd, buf);
        let _ = self.handle.trigger(cmd.priority(), cmd);
    }

    pub fn buffer_write(&mut self, buf: &[u8]) {
        self.out_buf.extend_from_slice(buf);
    }
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut total_read = 0;

        if let Some(in_buf) = self.in_buffer.as_mut() {
            let len = std::cmp::min(buf.len(), in_buf.len());
            if len > 0 {
                buf[..len].copy_from_slice(&in_buf[..len]);
                in_buf.advance(len);
                total_read += len;
            }
            if in_buf.is_empty() {
                self.in_buffer = None;
            }
        }

        if total_read == buf.len() {
            return Ok(total_read);
        }

        if let Some(io_buf) = self.io_buffer.as_mut() {
            let remaining = buf.len() - total_read;
            let len = std::cmp::min(remaining, io_buf.len());
            if len > 0 {
                buf[total_read..total_read + len].copy_from_slice(&io_buf[..len]);
                io_buf.advance(len);
                total_read += len;
            }
        }

        Ok(total_read)
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if !self.out_buf.is_empty() {
            self.out_buf.extend_from_slice(buf);
            return Ok(buf.len());
        }

        match self.socket.write(buf) {
            Ok(n) => {
                if n < buf.len() {
                    self.out_buf.extend_from_slice(&buf[n..]);
                }
                Ok(buf.len())
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.out_buf.extend_from_slice(buf);
                Ok(buf.len())
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                self.out_buf.extend_from_slice(buf);
                Ok(buf.len())
            }
            Err(e) => Err(e),
        }
    }

    // todo 饥饿读写
    fn flush(&mut self) -> std::io::Result<()> {
        while !self.out_buf.is_empty() {
            match self.socket.write(&self.out_buf) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "connection closed",
                    ));
                }
                Ok(n) => self.out_buf.advance(n),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        self.socket.flush()
    }
}

unsafe impl Send for Connection {}

unsafe impl Sync for Connection {}
