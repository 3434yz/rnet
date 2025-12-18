use bytes::BytesMut;

pub struct Context<'a> {
    pub local_addr: std::net::SocketAddr,
    pub peer_addr: std::net::SocketAddr,
    pub out_buf: &'a mut BytesMut,
}

pub enum Action<J> {
    None,
    Close,
    Publish(J),
    // Shutdown,
}

pub trait EventHandler: Send + Sync {
    type Job: Send + 'static;

    fn on_open(&self, ctx: &mut Context) -> Action<Self::Job>;

    fn on_traffic(&self, ctx: &mut Context, buf: &mut BytesMut) -> Action<Self::Job>;

    fn on_close(&self, ctx: &mut Context) -> Action<Self::Job>;
}
