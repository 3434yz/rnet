pub struct Request<J> {
    pub session_id: u64,
    pub job: J,
}

pub struct Response {
    pub session_id: u64,
    pub data: Vec<u8>,
}

pub fn pack_session_id(engine_id: usize, token: usize) -> u64 {
    ((engine_id as u64) << 32) | (token as u64)
}

pub fn unpack_session_id(session_id: u64) -> (usize, usize) {
    let engine_id = (session_id >> 32) as usize;
    let token = (session_id & 0xFFFF_FFFF) as usize;
    (engine_id, token)
}
