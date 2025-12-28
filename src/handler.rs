use crate::connection::Connection;
use crate::socket_addr::NetworkAddress;

use bytes::BytesMut;

pub struct Context<'a> {
    pub local_addr: NetworkAddress,
    pub peer_addr: NetworkAddress,
    pub out_buf: &'a mut BytesMut,
}

pub enum Action<J> {
    None,
    Close,
    Publish(J),
    // Shutdown,
}

pub trait EventHandler: Send + Sync + 'static{
    type Job: Send + Into<Vec<u8>> + 'static;

    fn on_open(&self, conn: &mut Connection) -> Action<Self::Job>;

    fn on_traffic(&self, conn: &mut Connection, cache: &mut BytesMut) -> Action<Self::Job>;

    fn on_close(&self, ctx: &mut Context) -> Action<Self::Job>;
}
