use crate::connection::Connection;

use bytes::BytesMut;

pub enum Action {
    None,
    Close,
    // Shutdown,
}

pub trait EventHandler: Send + Sync + 'static {
    fn on_open(&self, conn: &mut Connection) -> Action;

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action;

    fn on_close(&self, conn: &mut Connection) -> Action;
}
