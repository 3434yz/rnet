use bytes::BytesMut;

pub struct Context<'a> {
    pub local_addr: std::net::SocketAddr,
    pub peer_addr: std::net::SocketAddr,
    pub out_buf: &'a mut BytesMut,
}

pub enum Action {
    None,
    Close,
    // Shutdown,
}

pub trait EventHandler: Send + Sync {
    type Message;

    fn on_open(&self, ctx: &mut Context) -> Action;

    fn on_traffic(&self, ctx: &mut Context, msg: Self::Message) -> Action;

    fn on_close(&self, ctx: &mut Context) -> Action;
}
