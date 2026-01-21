pub(crate) struct Buffer {
    buffer: Vec<u8>,
    size: usize,
    r: usize, // next position to read
    w: usize, // next position to write
    empty: bool,
}

impl Buffer {
    pub(crate) fn new(mut size: usize) -> Self {
        size = size.next_power_of_two();

        Self {
            buffer: Vec::with_capacity(size),
            size,
            r: 0,
            w: 0,
            empty: true,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        unimplemented!()
    }

    // Reset the read pointer and write pointer to zero.
    pub(crate) fn reset(&mut self) {
        unimplemented!()
    }

    pub(crate) fn len(&self) -> usize {
        unimplemented!()
    }

    // Peek returns the next n bytes without advancing the read pointer,
    // it returns all bytes when n <= 0.
    pub(crate) fn peek(&self, n: usize) -> (&[u8], &[u8]) {
        unimplemented!()
    }

    // peekAll returns all bytes without advancing the read pointer.
    pub(crate) fn peek_all(&self) -> (&[u8], &[u8]) {
        unimplemented!()
    }

    // Discard skips the next n bytes by advancing the read pointer.
    pub(crate) fn discard(&mut self, n: usize) -> Option<usize> {
        unimplemented!()
    }

    pub(crate) fn read(&mut self, buf: &[u8]) -> Option<usize> {
        unimplemented!()
    }

    pub(crate) fn write(&mut self, buf: &[u8]) -> Option<usize> {
        unimplemented!()
    }
}
