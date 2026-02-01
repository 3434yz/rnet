use rnet::codec::Codec;
use rnet::connection::Connection;
use rnet::engine::EngineBuilder;
use rnet::handler::{Action, EventHandler};
use rnet::options::Options;

use bytes::{Buf, BufMut, BytesMut};
use mimalloc::MiMalloc;
use rnet::engine::EngineHandler;
use serde_json;

use std::any::Any;
use std::io;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const HEAD_SIZE: usize = 4;

pub struct JsonCodec;

impl Codec for JsonCodec {
    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Box<dyn Any + Send>>> {
        if src.len() < HEAD_SIZE {
            return Ok(None);
        }

        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if src.len() < HEAD_SIZE + len {
            return Ok(None);
        }

        src.advance(HEAD_SIZE);
        let body = src.split_to(len);

        let val: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Some(Box::new(val)))
    }

    fn encode(&mut self, msg: Box<dyn Any + Send>, dst: &mut BytesMut) -> io::Result<()> {
        let msg = msg.downcast::<serde_json::Value>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "JsonCodec expects serde_json::Value",
            )
        })?;
        let body =
            serde_json::to_vec(&msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        dst.reserve(HEAD_SIZE + body.len());
        dst.put_u32(body.len() as u32);
        dst.extend_from_slice(&body);
        Ok(())
    }
}

struct JsonHandler;

impl EventHandler for JsonHandler {
    fn init(
        _engine: std::sync::Arc<EngineHandler>,
        _options: std::sync::Arc<Options>,
    ) -> (Self, Action) {
        (JsonHandler, Action::None)
    }

    fn on_open(&self, conn: &mut Connection) -> Action {
        // 设置连接使用 JsonCodec
        conn.set_codec(JsonCodec);
        Action::None
    }

    fn on_traffic(&self, conn: &mut Connection) -> Action {
        while let Ok(Some(msg)) = conn.next_message() {
            if let Some(val) = msg.downcast_ref::<serde_json::Value>() {
                println!("Received JSON: {}", val);
                // 回显消息
                if let Err(e) = conn.send(val.clone()) {
                    eprintln!("Send error: {}", e);
                }
            }
        }
        Action::None
    }

    fn on_close(&self, _conn: &mut Connection) -> Action {
        Action::None
    }
}

fn main() {
    let mut options = Options::builder()
        .reuse_addr(true)
        .num_event_loop(4)
        .build();

    let addrs = vec!["tcp://127.0.0.1:9001"];
    let net_socket_addrs = options.normalize(&addrs).unwrap();

    let (mut engine, _) = EngineBuilder::builder()
        .address(net_socket_addrs)
        .build::<JsonHandler>(options)
        .unwrap();

    println!("Json Server running on tcp://127.0.0.1:9001");
    engine.run().unwrap();
}
