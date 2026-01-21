use std::{
    io::Read,
    ops::{Deref, DerefMut},
};

use bytes::{Buf, BytesMut};

pub struct IOBuffer {
    pub(crate) cursor: usize,
    pub(crate) reads: usize,
    pub(crate) buffer: Option<BytesMut>,
}

impl IOBuffer {
    pub fn new(mut size: usize) -> Self {
        size = size.next_power_of_two();
        let mut buffer = BytesMut::with_capacity(size);
        buffer.resize(size, 0);
        Self {
            buffer: Some(buffer),
            cursor: 0,
            reads: 0,
        }
    }

    pub fn remaining(&self) -> usize {
        debug_assert!(self.cursor <= self.reads);
        self.reads - self.cursor
    }

    pub fn remaining_bytes(&self) -> &[u8] {
        debug_assert!(self.buffer.is_some());
        debug_assert!(self.cursor <= self.reads);
        let Some(buffer) = self.buffer.as_ref() else {
            return &[];
        };
        &buffer[self.cursor..self.reads]
    }

    pub fn advance(&mut self, n: usize) {
        debug_assert!(self.remaining() >= n);
        self.cursor = self.cursor + n
    }

    pub fn set_cursor(&mut self, reads: usize) {
        debug_assert!(self.buffer.is_some());
        debug_assert!(reads <= self.buffer.as_ref().map(|b| b.capacity()).unwrap_or(0));
        self.cursor = 0;
        self.reads = reads;
    }

    pub(crate) fn split_to(&mut self, at: usize) -> BytesMut {
        debug_assert!(self.buffer.is_some());
        debug_assert!(at <= self.remaining());

        let buffer = self.buffer.as_mut().unwrap();

        if self.cursor > 0 {
            buffer.advance(self.cursor);
            self.reads -= self.cursor;
            self.cursor = 0;
        }

        let ret = buffer.split_to(at);
        self.reads -= at;
        ret
    }
}

impl Read for IOBuffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining_bytes = self.remaining_bytes();
        let n = std::cmp::min(buf.len(), remaining_bytes.len());
        buf[..n].copy_from_slice(&remaining_bytes[..n]);
        self.advance(n);
        Ok(n)
    }
}

impl Deref for IOBuffer {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.buffer.as_ref().unwrap()
    }
}

impl DerefMut for IOBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer.as_mut().unwrap()
    }
}
