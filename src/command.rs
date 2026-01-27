use bytes::Bytes;

use crate::{gfd::Gfd, socket::Socket, socket_addr::NetworkAddress};

pub enum Command {
    None,
    AsyncWrite(Gfd, Bytes),
    Register(Socket, NetworkAddress, NetworkAddress),
    Close(usize),
    Wake,
    Read(usize),
    Write(usize),
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
            Command::Read(_) => Prioity::High,
            Command::Write(_) => Prioity::High,
            Command::Shutdown => Prioity::High,
        }
    }
}

#[derive(PartialEq, PartialOrd)]
pub enum Prioity {
    High,
    Low,
}
