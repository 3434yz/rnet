use crate::{connection::Connection, engine::EngineHandler};

use bytes::BytesMut;

use std::sync::Arc;

pub enum Action {
    None,
    Close,
    Shutdown, // 发送shutdown command
}

pub trait EventHandler: Send + Sync + 'static {
    // fn on_boot(engine: Arc<EngineHandler>) -> Action;

    fn on_open(&self, conn: &mut Connection) -> Action;

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action;

    fn on_close(&self, conn: &mut Connection) -> Action;
}
