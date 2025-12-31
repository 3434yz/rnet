use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

static MONO_SEQ: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Gfd {
    seq: u32,
    index_info: u32,
    fd: i64,
}

impl Gfd {
    #[inline]
    pub fn new(fd: i64, loop_id: u8, token: usize) -> Self {
        debug_assert!(token < (1 << 24), "Slab index overflow");

        let seq = MONO_SEQ.fetch_add(1, Ordering::Relaxed);

        let index_info = ((loop_id as u32) << 24) | (token as u32 & 0xFFFFFF);

        Self {
            fd,
            seq,
            index_info,
        }
    }

    #[inline]
    pub fn fd(&self) -> i64 {
        self.fd
    }

    #[inline]
    pub fn sequence(&self) -> u32 {
        self.seq
    }

    #[inline]
    pub fn event_loop_index(&self) -> usize {
        (self.index_info >> 24) as usize
    }

    #[inline]
    pub fn slab_index(&self) -> usize {
        (self.index_info & 0xFFFFFF) as usize
    }

    pub fn validate(&self) -> bool {
        self.fd >= 0 && self.seq > 0
    }
}

impl fmt::Debug for Gfd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Gfd {{ fd: {}, seq: {}, loop: {}, index: {} }}",
            self.fd,
            self.sequence(),
            self.event_loop_index(),
            self.slab_index()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gfd_layout() {
        // 验证结构体大小确实是 16 字节
        assert_eq!(std::mem::size_of::<Gfd>(), 16);
        assert_eq!(std::mem::align_of::<Gfd>(), 8);
    }

    #[test]
    fn test_gfd_packing() {
        let fd = 100;
        let loop_id = 5;
        let slab_index = 12345;

        let gfd = Gfd::new(fd, loop_id, slab_index);

        assert_eq!(gfd.fd(), 100);
        assert_eq!(gfd.event_loop_index(), 5);
        assert_eq!(gfd.slab_index(), 12345);

        // 验证序列号递增
        let gfd2 = Gfd::new(fd, loop_id, slab_index);
        assert_ne!(gfd.sequence(), gfd2.sequence());
        assert_eq!(gfd.sequence() + 1, gfd2.sequence());
    }
}
