use bytes::BytesMut;
use std::any::Any;
use std::io;

pub trait Codec: Send {
    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Box<dyn Any + Send>>>;
    fn encode(&mut self, msg: Box<dyn Any + Send>, dst: &mut BytesMut) -> io::Result<()>;
}

#[derive(Clone, Debug)]
pub struct RawCodec;

impl Codec for RawCodec {
    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Box<dyn Any + Send>>> {
        if src.is_empty() {
            return Ok(None);
        }
        let len = src.len();
        Ok(Some(Box::new(src.split_to(len))))
    }

    fn encode(&mut self, msg: Box<dyn Any + Send>, dst: &mut BytesMut) -> io::Result<()> {
        let msg = msg.downcast::<BytesMut>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "RawCodec expects BytesMut message",
            )
        })?;
        dst.extend_from_slice(&msg);
        Ok(())
    }
}
