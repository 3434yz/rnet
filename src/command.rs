use crate::gfd::Gfd;

pub struct Request<J> {
    pub gfd: Gfd,
    pub job: J,
}

pub struct Response {
    pub gfd: Gfd,
    pub data: Vec<u8>,
}
