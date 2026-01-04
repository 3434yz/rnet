use crate::{gfd::Gfd, socket::Socket, socket_addr::NetworkAddress};

pub enum Command<J> {
    JobReq(Gfd, J),
    JobResp(Gfd, Vec<u8>),
    Register(Socket, NetworkAddress, NetworkAddress),
    Close(usize),
    Wake(),
    Read(usize),
    Write(usize),
}
