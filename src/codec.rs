use bytes::{Buf, BufMut, BytesMut};
use std::io;

use crate::connection::Connection;
use serde_json;

pub trait Codec {
    type Message;

    fn decode(
        &mut self,
        conn: &mut Connection,
        cache: &mut BytesMut,
    ) -> io::Result<Option<Self::Message>>;

    fn encode(&mut self, msg: Self::Message, buf: &mut BytesMut) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug)]
pub struct RawCodec;

impl Codec for RawCodec {
    type Message = BytesMut;

    fn decode(
        &mut self,
        conn: &mut Connection,
        cache: &mut BytesMut,
    ) -> io::Result<Option<Self::Message>> {
        if conn.in_buf.is_empty() {
            return Ok(None);
        }
        // 将所有现有数据全部取走
        let len = conn.in_buf.len();
        Ok(Some(conn.in_buf.split_to(len)))
    }

    fn encode(&mut self, msg: Self::Message, cache: &mut BytesMut) -> io::Result<()> {
        cache.extend_from_slice(&msg);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LengthDelimitedCodec;

impl Codec for LengthDelimitedCodec {
    type Message = Vec<u8>;

    fn decode(
        &mut self,
        conn: &mut Connection,
        buf: &mut BytesMut,
    ) -> io::Result<Option<Self::Message>> {
        if buf.len() < 4 {
            return Ok(None);
        }

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

const MAGIC_NUMBER: usize = 1314;
const MAGIC_NUMBER_SIZE: usize = 2;
const BODY_SIZE: usize = 4;

pub struct JsonCodec {}

impl Codec for JsonCodec {
    type Message = serde_json::Value;

    fn decode(
        &mut self,
        conn: &mut Connection,
        cache: &mut BytesMut,
    ) -> io::Result<Option<Self::Message>> {
        let header_size = MAGIC_NUMBER_SIZE + BODY_SIZE;
        if conn.remaining() < header_size {
            return Ok(None);
        }

        let (magic, len) = {
            let header = conn.zpeek(Some(header_size), cache).unwrap();
            let magic = u16::from_be_bytes([header[0], header[1]]);
            let len = u32::from_be_bytes([header[2], header[3], header[4], header[5]]) as usize;
            (magic, len)
        };

        if magic as usize != MAGIC_NUMBER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid magic number",
            ));
        }

        if conn.remaining() < header_size + len {
            return Ok(None);
        }

        conn.discard(Some(header_size));
        let body = conn.next(Some(len), cache).unwrap();

        let val = serde_json::from_slice(body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Some(val))
    }

    fn encode(&mut self, msg: Self::Message, cache: &mut BytesMut) -> io::Result<()> {
        let body =
            serde_json::to_vec(&msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        cache.reserve(MAGIC_NUMBER_SIZE + BODY_SIZE + body.len());
        cache.put_u16(MAGIC_NUMBER as u16);
        cache.put_u32(body.len() as u32);
        cache.extend_from_slice(&body);
        Ok(())
    }
}
