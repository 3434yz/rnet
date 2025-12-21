use bytes::BytesMut;
use mio::Poll;

use crate::buffer;

struct BytesPool {
    buffer: BytesMut,
}

impl BytesPool {
    fn new(n: usize) -> Self {
        Self {
            buffer: BytesMut::with_capacity(n),
        }
    }
}

#[cfg(test)]
mod test {
    use bytes::{BufMut, BytesMut};

    #[test]
    fn it_works() {
        let mut buf = BytesMut::with_capacity(1024);

        unsafe {
            let src = b"Hello";
            let len_to_write = src.len(); // 5

            let spare_capacity = buf.chunk_mut();
            let dst_ptr = spare_capacity.as_mut_ptr();

            std::ptr::copy_nonoverlapping(src.as_ptr(), dst_ptr, len_to_write);
            buf.advance_mut(len_to_write);
        }

        // 验证
        assert_eq!(&buf[..], b"Hello");
        println!("Buf content: {:?}", std::str::from_utf8(&buf).unwrap());
    }
}
