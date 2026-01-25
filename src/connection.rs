use crate::command::Command;
use crate::event_loop::EventLoopHandle;
use crate::gfd::Gfd;
use crate::options::Options;
use crate::socket::Socket;
use crate::socket_addr::NetworkAddress;

use bytes::{Buf, Bytes, BytesMut};
use smallvec::SmallVec;

use std::cell::RefCell;
use std::collections::VecDeque;
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

const IO_MAX: usize = 1024;

thread_local! {
    static IO_SLICES: RefCell<Vec<std::io::IoSlice<'static>>> = RefCell::new(Vec::with_capacity(IO_MAX));
}

pub struct Connection {
    pub(crate) gfd: Gfd,
    pub(crate) socket: Socket,
    pub(crate) local_addr: NetworkAddress,
    pub(crate) peer_addr: NetworkAddress,
    pub(crate) closed: bool,
    pub(crate) io_buffer: Option<BytesMut>,
    pub(crate) in_buffer: Option<BytesMut>,
    pub(crate) out_buffer: Option<VecDeque<Bytes>>,
    pub(crate) handle: Arc<EventLoopHandle>,
    pub(crate) options: Arc<Options>,
    pub codec: Option<Box<dyn crate::codec::Codec>>,
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
            out_buffer: None,
            handle,
            options,
            codec: None,
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

    pub fn next_contiguous(&mut self, n: Option<usize>) -> Option<BytesMut> {
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

    pub fn next_vectored(&mut self, n: Option<usize>) -> Option<SmallVec<[BytesMut; 2]>> {
        let in_buffer_len = self.in_buf_len();
        let io_buffer_len = self.io_buf_len();
        let total_len = in_buffer_len + io_buffer_len;

        let actual_len = match n {
            Some(n) if n > total_len || n <= 0 => return None,
            Some(n) => n,
            None => total_len,
        };

        let mut remaining = actual_len;
        let mut result = SmallVec::new();

        if let Some(in_buf_mut) = self.in_buffer.as_mut() {
            let len = std::cmp::min(remaining, in_buf_mut.len());
            if len > 0 {
                let data = in_buf_mut.split_to(len);
                result.push(data);
                remaining -= len;
            }
            if in_buf_mut.is_empty() {
                self.in_buffer = None;
            }
        }

        if remaining > 0 {
            if let Some(io_buf_mut) = self.io_buffer.as_mut() {
                let data = io_buf_mut.split_to(remaining);
                result.push(data);
            }
        }

        Some(result)
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

    pub fn set_codec<C: crate::codec::Codec + 'static>(&mut self, codec: C) {
        self.codec = Some(Box::new(codec));
    }

    pub fn next_message(&mut self) -> std::io::Result<Option<Box<dyn std::any::Any + Send>>> {
        if let Some(codec) = self.codec.as_mut() {
            if let Some(in_buffer) = self.in_buffer.as_mut() {
                return codec.decode(in_buffer);
            }
        }
        Ok(None)
    }

    pub fn send<T: 'static + Send>(&mut self, msg: T) -> std::io::Result<()> {
        if let Some(codec) = self.codec.as_mut() {
            let mut buf = BytesMut::new();
            codec.encode(Box::new(msg), &mut buf)?;
            let bytes = buf.freeze();
            self.write_bytes(bytes)?;
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "No codec set",
            ))
        }
    }

    pub fn write_bytes(&mut self, buf: Bytes) -> std::io::Result<usize> {
        let len = buf.len();
        if let Some(out_buffer) = &mut self.out_buffer {
            if !out_buffer.is_empty() {
                out_buffer.push_back(buf);
                return Ok(len);
            }
        }

        match self.socket.write(&buf) {
            Ok(n) => {
                if n < len {
                    let remaining = buf.slice(n..);
                    self.out_buffer
                        .get_or_insert_with(VecDeque::new)
                        .push_back(remaining);
                }
                Ok(len)
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::Interrupted =>
            {
                self.out_buffer
                    .get_or_insert_with(VecDeque::new)
                    .push_back(buf);
                Ok(len)
            }
            Err(e) => Err(e),
        }
    }

    pub fn write_vectored_bytes(&mut self, bufs: &[Bytes]) -> std::io::Result<usize> {
        let total_len: usize = bufs.iter().map(|b| b.len()).sum();
        if total_len == 0 {
            return Ok(0);
        }

        if let Some(out_buffer) = &mut self.out_buffer {
            if !out_buffer.is_empty() {
                out_buffer.extend(bufs.iter().cloned());
                return Ok(total_len);
            }
        }

        let res = IO_SLICES.with(|cells| {
            let mut io_slices = cells.borrow_mut();
            io_slices.clear();
            for buf in bufs {
                let slice = std::io::IoSlice::new(buf);
                let static_slice = unsafe {
                    std::mem::transmute::<std::io::IoSlice<'_>, std::io::IoSlice<'static>>(slice)
                };
                io_slices.push(static_slice);
            }
            self.socket.write_vectored(&io_slices)
        });

        match res {
            Ok(n) => {
                if n < total_len {
                    let mut written = n;
                    let out_buffer = self.out_buffer.get_or_insert_with(VecDeque::new);
                    for buf in bufs {
                        let len = buf.len();
                        if written >= len {
                            written -= len;
                        } else {
                            if written > 0 {
                                out_buffer.push_back(buf.slice(written..));
                                written = 0;
                            } else {
                                out_buffer.push_back(buf.clone());
                            }
                        }
                    }
                }
                Ok(total_len)
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::Interrupted =>
            {
                self.out_buffer
                    .get_or_insert_with(VecDeque::new)
                    .extend(bufs.iter().cloned());
                Ok(total_len)
            }
            Err(e) => Err(e),
        }
    }

    pub fn close(&mut self) {
        let cmd = Command::Close(self.gfd.slab_index());
        let _ = self.handle.trigger(cmd.priority(), cmd);
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
        if let Some(out_buffer) = &mut self.out_buffer {
            if !out_buffer.is_empty() {
                out_buffer.push_back(Bytes::copy_from_slice(buf));
                return Ok(buf.len());
            }
        }

        match self.socket.write(buf) {
            Ok(n) => {
                if n < buf.len() {
                    let remaining = Bytes::copy_from_slice(&buf[n..]);
                    self.out_buffer
                        .get_or_insert_with(VecDeque::new)
                        .push_back(remaining);
                }
                Ok(buf.len())
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let bytes = Bytes::copy_from_slice(buf);
                self.out_buffer
                    .get_or_insert_with(VecDeque::new)
                    .push_back(bytes);
                Ok(buf.len())
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                let bytes = Bytes::copy_from_slice(buf);
                self.out_buffer
                    .get_or_insert_with(VecDeque::new)
                    .push_back(bytes);
                Ok(buf.len())
            }
            Err(e) => Err(e),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(out_buffer) = &mut self.out_buffer {
            let socket = &mut self.socket;
            IO_SLICES.with(|cells| {
                let mut io_slices = cells.borrow_mut();
                while !out_buffer.is_empty() {
                    io_slices.clear();
                    for buf in out_buffer.iter().take(IO_MAX) {
                        let slice = std::io::IoSlice::new(buf);
                        let static_slice = unsafe {
                            std::mem::transmute::<std::io::IoSlice<'_>, std::io::IoSlice<'static>>(
                                slice,
                            )
                        };
                        io_slices.push(static_slice);
                    }

                    let res = socket.write_vectored(&io_slices);
                    io_slices.clear();

                    match res {
                        Ok(0) => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::WriteZero,
                                "connection closed",
                            ));
                        }
                        Ok(n) => {
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
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::WouldBlock,
                                "flush would block",
                            ));
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(())
            })?;
        }
        self.socket.flush()
    }

    fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
        let total_len: usize = bufs.iter().map(|b| b.len()).sum();
        if total_len == 0 {
            return Ok(0);
        }

        if let Some(out_buffer) = &mut self.out_buffer {
            if !out_buffer.is_empty() {
                for buf in bufs {
                    out_buffer.push_back(Bytes::copy_from_slice(buf));
                }
                return Ok(total_len);
            }
        }

        match self.socket.write_vectored(bufs) {
            Ok(n) => {
                if n < total_len {
                    let mut written = n;
                    let out_buffer = self.out_buffer.get_or_insert_with(VecDeque::new);
                    for buf in bufs {
                        let len = buf.len();
                        if written >= len {
                            written -= len;
                        } else {
                            out_buffer.push_back(Bytes::copy_from_slice(&buf[written..]));
                            written = 0;
                        }
                    }
                }
                Ok(total_len)
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::Interrupted =>
            {
                let out_buffer = self.out_buffer.get_or_insert_with(VecDeque::new);
                for buf in bufs {
                    out_buffer.push_back(Bytes::copy_from_slice(buf));
                }
                Ok(total_len)
            }
            Err(e) => Err(e),
        }
    }
}
