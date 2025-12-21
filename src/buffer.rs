use std::ops::{Deref, DerefMut};

pub struct IOBuffer {
    pub(crate) cursor: usize,
    pub(crate) reads: usize,
    pub(crate) buffer: Vec<u8>,
}

impl IOBuffer {
    pub fn new(size: usize) -> Self {
        let mut buffer = Vec::with_capacity(size);
        buffer.resize(size, 0);
        Self {
            buffer,
            cursor: 0,
            reads: 0,
        }
    }

    pub fn remaining(&self) -> usize {
        debug_assert!(self.cursor <= self.reads);
        self.reads - self.cursor
    }

    pub fn remaining_bytes(&self) -> &[u8] {
        debug_assert!(self.cursor <= self.reads);
        &self.buffer[self.cursor..self.reads]
    }

    pub fn advance(&mut self, n: usize) {
        debug_assert!(self.remaining() >= n);
        self.cursor = self.cursor + n
    }

    pub fn read(&mut self, reads: usize) {
        debug_assert!(reads <= self.buffer.capacity());
        self.cursor = 0;
        self.reads = reads;
    }
}

impl Deref for IOBuffer {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl DerefMut for IOBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
    }
}
