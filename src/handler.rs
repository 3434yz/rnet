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
    // 来自 TCP 的消息类型 (由 Codec 产出)
    type Message;
    type Job: Send + 'static;

    fn on_open(&self, ctx: &mut Context) -> Action<Self::Job>;

    fn on_message(&self, ctx: &mut Context, msg: Self::Message) -> Action<Self::Job>;

    fn on_close(&self, ctx: &mut Context) -> Action<Self::Job>;
}
