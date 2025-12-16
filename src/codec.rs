use bytes::{Buf, BufMut, BytesMut};
use std::io;

pub trait Codec {
    type Message;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Message>>;

    fn encode(&mut self, msg: Self::Message, buf: &mut BytesMut) -> io::Result<()>;
}

// === 实现 1: RawCodec (透传模式) ===
// 这个模式下，行为和之前完全一样：来多少读多少。用于保持 122w QPS 压测。
#[derive(Clone, Copy, Debug)]
pub struct RawCodec;

impl Codec for RawCodec {
    type Message = BytesMut;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Message>> {
        if buf.is_empty() {
            return Ok(None);
        }
        // 将所有现有数据全部取走
        let len = buf.len();
        Ok(Some(buf.split_to(len)))
    }

    fn encode(&mut self, msg: Self::Message, buf: &mut BytesMut) -> io::Result<()> {
        buf.extend_from_slice(&msg);
        Ok(())
    }
}

// === 实现 2: LengthDelimitedCodec (4字节头模式) ===
// 格式: [Length: u32 big-endian][Body...]
#[derive(Clone, Copy, Debug)]
pub struct LengthDelimitedCodec;

impl Codec for LengthDelimitedCodec {
    type Message = Vec<u8>;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Message>> {
        if buf.len() < 4 {
            return Ok(None);
        }

        // Peek 长度
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&buf[..4]);
        let len = u32::from_be_bytes(len_bytes) as usize;

        if buf.len() < 4 + len {
            return Ok(None);
        }

        // 切割
        buf.advance(4);
        let body = buf.split_to(len).to_vec();
        Ok(Some(body))
    }

    fn encode(&mut self, msg: Self::Message, buf: &mut BytesMut) -> io::Result<()> {
        buf.reserve(4 + msg.len());
        buf.put_u32(msg.len() as u32);
        buf.put_slice(&msg);
        Ok(())
    }
}
