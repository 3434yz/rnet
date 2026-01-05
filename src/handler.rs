use crate::connection::Connection;

use bytes::BytesMut;

pub enum Action<J> {
    None,
    Close,
    Submit(J),
    // Shutdown,
}

pub trait EventHandler: Send + Sync + 'static {
    type Job: Send + Into<Vec<u8>> + 'static;
    fn on_open(&self, conn: &mut Connection) -> Action<Self::Job>;

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action<Self::Job>;

    fn on_close(&self, conn: &mut Connection) -> Action<Self::Job>;
}
