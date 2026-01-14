use crate::command::Command;
use crate::event_loop::EventLoopHandle;
use crate::gfd::Gfd;
use crate::io_buffer::IOBuffer;
use crate::options::Options;
use crate::socket::Socket;
use crate::socket_addr::NetworkAddress;

use bytes::{Buf, Bytes, BytesMut};

use std::io::{Read, Write};
use std::ptr::NonNull;
use std::sync::Arc;

pub struct Connection {
    pub(crate) gfd: Gfd,
    pub(crate) socket: Socket,
    pub(crate) local_addr: NetworkAddress,
    pub(crate) peer_addr: NetworkAddress,
    pub(crate) closed: bool,
    pub(crate) in_buf: BytesMut,
    pub(crate) out_buf: BytesMut,
    pub(crate) handle: Arc<EventLoopHandle>,
    buf_ptr: NonNull<IOBuffer>,
}

impl Connection {
    pub fn new(
        gfd: Gfd,
        socket: Socket,
        options: Arc<Options>,
        local_addr: NetworkAddress,
        peer_addr: NetworkAddress,
        handle: Arc<EventLoopHandle>,
        buf_ptr: NonNull<IOBuffer>,
    ) -> Self {
        Self {
            gfd,
            socket,
            local_addr,
            peer_addr,
            closed: false,
            in_buf: BytesMut::with_capacity(options.read_buffer_cap),
            out_buf: BytesMut::with_capacity(options.write_buffer_cap),
            handle,
            buf_ptr,
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

    pub fn remaining(&self) -> usize {
        unsafe {
            let buffer = self.buf_ptr.as_ref();
            self.in_buf.len() + buffer.remaining()
        }
    }

    pub fn znext<'a>(&mut self, n: Option<usize>, cache: &'a mut BytesMut) -> Option<&'a [u8]> {
        unsafe {
            let el_buf = self.buf_ptr.as_mut();
            let in_len = self.in_buf.len();
            let el_len = el_buf.remaining();
            let total_len = in_len + el_len;

            if total_len == 0 {
                return None;
            }

            let actual_len = match n {
                Some(n) if n > total_len => return None,
                Some(n) => n,
                None => total_len,
            };

            if in_len == 0 {
                let ptr = el_buf.buffer.as_ptr().add(el_buf.cursor);
                let slice = std::slice::from_raw_parts(ptr, actual_len);
                el_buf.advance(actual_len);
                return Some(slice);
            }

            let take_from_in = std::cmp::min(in_len, actual_len);
            cache.clear();
            cache.extend_from_slice(&self.in_buf.split_to(take_from_in));

            let take_from_el = actual_len - take_from_in;
            if take_from_el > 0 {
                let ptr = el_buf.buffer.as_ptr().add(el_buf.cursor);
                let slice = std::slice::from_raw_parts(ptr, take_from_el);
                cache.extend_from_slice(slice);
                el_buf.advance(take_from_el);
            }

            Some(&cache[..])
        }
    }

    pub fn next(&mut self, n: Option<usize>, cache: &mut BytesMut) -> Option<BytesMut> {
        unsafe {
            let el_buf = self.buf_ptr.as_mut();
            let in_len = self.in_buf.len();
            let el_len = el_buf.remaining();
            let total_len = in_len + el_len;

            if total_len == 0 {
                return None;
            }

            let actual_len = match n {
                Some(n) if n > total_len => return None,
                Some(n) => n,
                None => total_len,
            };

            if in_len >= actual_len {
                return Some(self.in_buf.split_to(actual_len));
            }

            cache.clear();
            if in_len > 0 {
                cache.extend_from_slice(&self.in_buf);
                self.in_buf.clear();
            }

            let remaining = actual_len - in_len;
            if remaining > 0 {
                let ptr = el_buf.buffer.as_ptr().add(el_buf.cursor);
                let slice = std::slice::from_raw_parts(ptr, remaining);
                cache.extend_from_slice(slice);
                el_buf.advance(remaining);
            }

            Some(cache.split_to(actual_len))
        }
    }

    pub fn zpeek<'a>(&'a self, n: Option<usize>, cache: &'a mut BytesMut) -> Option<&'a [u8]> {
        unsafe {
            let el_buf = self.buf_ptr.as_ref();
            let in_len = self.in_buf.len();
            let el_len = el_buf.remaining();
            let total_len = in_len + el_len;

            if total_len == 0 {
                return None;
            }

            let actual_len = match n {
                Some(n) if n > total_len => return None,
                Some(n) => n,
                None => total_len,
            };

            if in_len == 0 {
                let ptr = el_buf.buffer.as_ptr().add(el_buf.cursor);
                let slice = std::slice::from_raw_parts(ptr, actual_len);
                return Some(slice);
            }

            if in_len >= actual_len {
                return Some(&self.in_buf[..actual_len]);
            }

            cache.clear();
            cache.extend_from_slice(&self.in_buf);
            let remaining = actual_len - in_len;
            if remaining > 0 {
                let slice = &el_buf.remaining_bytes()[..remaining];
                cache.extend_from_slice(slice);
            }
            return Some(&cache[..]);
        }
    }

    pub fn peek<'a>(&self, n: Option<usize>, cache: &'a mut BytesMut) -> Option<&'a [u8]> {
        unsafe {
            let el_buf = self.buf_ptr.as_ref();
            let in_len = self.in_buf.len();
            let el_len = el_buf.remaining();
            let total_len = in_len + el_len;

            if total_len == 0 {
                return None;
            }

            let actual_len = match n {
                Some(n) if n > total_len => return None,
                Some(n) => n,
                None => total_len,
            };

            if in_len == 0 {
                let ptr = el_buf.buffer.as_ptr().add(el_buf.cursor);
                let slice = std::slice::from_raw_parts(ptr, actual_len);
                return Some(slice);
            }

            cache.clear();
            if in_len >= actual_len {
                cache.extend_from_slice(&self.in_buf[..actual_len]);
                return Some(&cache[..]);
            }

            cache.extend_from_slice(&self.in_buf);
            let remaining = actual_len - in_len;
            if remaining > 0 {
                let slice = &el_buf.remaining_bytes()[..remaining];
                cache.extend_from_slice(slice);
            }
            return Some(&cache[..]);
        }
    }

    pub fn discard(&mut self, n: Option<usize>) -> Option<usize> {
        unsafe {
            let el_buf = self.buf_ptr.as_mut();
            let in_len = self.in_buf.len();
            let el_len = el_buf.remaining();
            let total_len = in_len + el_len;

            if total_len == 0 {
                return None;
            }

            let actual_len = match n {
                Some(n) if n > total_len => return None,
                Some(n) => n,
                None => total_len,
            };

            if in_len >= actual_len {
                self.in_buf.advance(actual_len);
            } else {
                self.in_buf.advance(in_len);
                let remaining = actual_len - in_len;
                el_buf.advance(remaining);
            }

            Some(actual_len)
        }
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
        let mut written = 0;

        if !self.in_buf.is_empty() {
            let len = std::cmp::min(buf.len(), self.in_buf.len());
            buf[..len].copy_from_slice(&self.in_buf[..len]);
            self.in_buf.advance(len);
            written += len;
        }

        if written == buf.len() {
            return Ok(written);
        }

        unsafe {
            let el_buf = self.buf_ptr.as_mut();
            let remaining = el_buf.remaining();
            if remaining > 0 {
                let to_read = std::cmp::min(buf.len() - written, remaining);

                let ptr = el_buf.buffer.as_ptr().add(el_buf.cursor);
                let slice = std::slice::from_raw_parts(ptr, to_read);

                buf[written..written + to_read].copy_from_slice(slice);
                el_buf.advance(to_read);
                written += to_read;
            }
        }

        Ok(written)
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
