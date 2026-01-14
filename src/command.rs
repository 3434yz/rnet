use bytes::Bytes;

use crate::{gfd::Gfd, socket::Socket, socket_addr::NetworkAddress};

pub enum Command {
    None,
    AsyncWrite(Gfd, Bytes),
    Register(Socket, NetworkAddress, NetworkAddress),
    Close(usize),
    Wake,
    IORead(usize),
    IOWrite(usize),
    Shutdown,
}

impl Command {
    pub fn priority(&self) -> Prioity {
        match self {
            Command::None => Prioity::Low,
            Command::AsyncWrite(_, _) => Prioity::High,
            Command::Wake => Prioity::Low,
            Command::Register(_, _, _) => Prioity::Low,
            Command::Close(_) => Prioity::Low,
            Command::IORead(_) => Prioity::High,
            Command::IOWrite(_) => Prioity::High,
            Command::Shutdown => Prioity::High,
        }
    }
}

#[derive(PartialEq, PartialOrd)]
pub enum Prioity {
    High,
    Low,
}
