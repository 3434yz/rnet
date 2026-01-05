use crate::{gfd::Gfd, socket::Socket, socket_addr::NetworkAddress};

pub enum Command {
    None,
    Write(Gfd, Vec<u8>),
    Register(Socket, NetworkAddress, NetworkAddress),
    Close(usize),
    Wake(),
    IORead(usize),
    IOWrite(usize),
}
