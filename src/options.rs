use std::io;
use std::time::Duration;
use crate::socket_addr::NetworkAddress;

const DEFAULT_BUFFER_SIZE: usize = 1024;
const MAX_STREAM_BUFFER_CAP: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalancing {
    RoundRobin,
    LeastConnections,
    SourceAddrHash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Default for Level {
    fn default() -> Self {
        Level::Info
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpSocketOpt {
    NoDelay,
    Delay,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub lb: LoadBalancing,
    pub reuse_addr: bool,
    pub reuse_port: bool,
    // pub multicast_interface_index: u32,
    pub bind_to_device: String,
    pub multicore: bool,
    pub num_event_loop: usize,
    pub read_buffer_cap: usize,
    pub write_buffer_cap: usize,
    pub lock_os_thread: bool,
    pub ticker: bool,
    pub tcp_keepalive: Option<Duration>,
    pub tcp_keep_interval: Option<Duration>,
    pub tcp_keep_count: Option<u32>,
    pub tcp_no_delay: TcpSocketOpt,
    pub socket_recv_buffer: usize,
    pub socket_send_buffer: usize,
    pub edge_triggered_io_chunk: usize,

    // === 日志相关 ===
    pub log_path: Option<String>,
    pub log_level: Level,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            lb: LoadBalancing::RoundRobin,
            reuse_addr: false,
            reuse_port: false,
            // multicast_interface_index: 0,
            bind_to_device: String::new(),
            multicore: false,
            num_event_loop: 0,
            read_buffer_cap: MAX_STREAM_BUFFER_CAP,
            write_buffer_cap: MAX_STREAM_BUFFER_CAP,
            lock_os_thread: false,
            ticker: false,
            tcp_keepalive: None,
            tcp_keep_interval: None,
            tcp_keep_count: None,
            tcp_no_delay: TcpSocketOpt::NoDelay,
            socket_recv_buffer: 0,
            socket_send_buffer: 0,
            edge_triggered_io_chunk: 0,

            log_path: None,
            log_level: Level::default(),
        }
    }
}

impl Options {
    pub fn builder() -> OptionsBuilder {
        OptionsBuilder::new()
    }

    pub fn normalize(&mut self, addrs: &[impl AsRef<str>]) -> io::Result<Vec<NetworkAddress>> {
        let mut parsed_addrs = Vec::with_capacity(addrs.len());
        // let mut has_udp = false;
        let mut has_unix = false;

        for addr in addrs {
            let net_addr: NetworkAddress = match addr.as_ref().parse() {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("Invalid address format '{}': {}", addr.as_ref(), e);
                    return Err(e);
                }
            };

            match net_addr {
                // NetworkAddress::Udp(_) => has_udp = true,
                #[cfg(unix)]
                NetworkAddress::Unix(_) => has_unix = true,
                _ => {} // TCP
            }

            parsed_addrs.push(net_addr);
        }

        if self.reuse_port && (self.multicore || self.num_event_loop > 1) {
            eprintln!(
                "rnet: SO_REUSEPORT is disabled on this platform for multicore mode, falling back to master-slave reactor."
            );
            self.reuse_port = false;
        }

        if self.reuse_port && has_unix {
            self.reuse_port = false;
        }

        // if has_udp {
        //     self.reuse_port = true;
        // }

        Ok(parsed_addrs)
    }
}

#[derive(Debug, Default)]
pub struct OptionsBuilder {
    base_options: Option<Options>,

    lb: Option<LoadBalancing>,
    reuse_addr: Option<bool>,
    reuse_port: Option<bool>,
    // multicast_interface_index: Option<u32>,
    bind_to_device: Option<String>,
    multicore: Option<bool>,
    num_event_loop: Option<usize>,
    read_buffer_cap: Option<usize>,
    write_buffer_cap: Option<usize>,
    lock_os_thread: Option<bool>,
    ticker: Option<bool>,
    tcp_keepalive: Option<Duration>,
    tcp_keep_interval: Option<Duration>,
    tcp_keep_count: Option<u32>,
    tcp_no_delay: Option<TcpSocketOpt>,
    socket_recv_buffer: Option<usize>,
    socket_send_buffer: Option<usize>,
    edge_triggered_io_chunk: Option<usize>,
    log_path: Option<String>,
    log_level: Option<Level>,
}

impl OptionsBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(self) -> Options {
        let mut opts = self.base_options.unwrap_or_else(Options::default);

        if let Some(v) = self.lb {
            opts.lb = v;
        }
        if let Some(v) = self.reuse_addr {
            opts.reuse_addr = v;
        }
        if let Some(v) = self.reuse_port {
            opts.reuse_port = v;
        }
        // if let Some(v) = self.multicast_interface_index {
        //     opts.multicast_interface_index = v;
        // }
        if let Some(v) = self.bind_to_device {
            opts.bind_to_device = v;
        }
        if let Some(v) = self.multicore {
            opts.multicore = v;
        }
        if let Some(v) = self.num_event_loop {
            opts.num_event_loop = v;
        }
        if let Some(v) = self.read_buffer_cap {
            opts.read_buffer_cap = v;
        }
        if let Some(v) = self.write_buffer_cap {
            opts.write_buffer_cap = v;
        }
        if let Some(v) = self.lock_os_thread {
            opts.lock_os_thread = v;
        }
        if let Some(v) = self.ticker {
            opts.ticker = v;
        }
        if let Some(v) = self.tcp_keepalive {
            opts.tcp_keepalive = Some(v);
        }
        if let Some(v) = self.tcp_keep_interval {
            opts.tcp_keep_interval = Some(v);
        }
        if let Some(v) = self.tcp_keep_count {
            opts.tcp_keep_count = Some(v);
        }
        if let Some(v) = self.tcp_no_delay {
            opts.tcp_no_delay = v;
        }
        if let Some(v) = self.socket_recv_buffer {
            opts.socket_recv_buffer = v;
        }
        if let Some(v) = self.socket_send_buffer {
            opts.socket_send_buffer = v;
        }
        if let Some(v) = self.edge_triggered_io_chunk {
            opts.edge_triggered_io_chunk = v;
        }
        if let Some(v) = self.log_path {
            opts.log_path = Some(v);
        }
        if let Some(v) = self.log_level {
            opts.log_level = v;
        }

        if opts.edge_triggered_io_chunk > 0 {
            opts.edge_triggered_io_chunk = ceil_to_power_of_two(opts.edge_triggered_io_chunk);
        } else  {
            opts.edge_triggered_io_chunk = 1 << 20;
        }

        opts.read_buffer_cap = normalize_buffer_size(opts.read_buffer_cap);
        opts.write_buffer_cap = normalize_buffer_size(opts.write_buffer_cap);
        opts
    }

    pub fn with_options(mut self, options: Options) -> Self {
        self.base_options = Some(options);
        self
    }
    pub fn lb(mut self, val: LoadBalancing) -> Self {
        self.lb = Some(val);
        self
    }
    pub fn reuse_addr(mut self, val: bool) -> Self {
        self.reuse_addr = Some(val);
        self
    }
    pub fn reuse_port(mut self, val: bool) -> Self {
        self.reuse_port = Some(val);
        self
    }
    // pub fn multicast_interface_index(mut self, val: u32) -> Self {
    //     self.multicast_interface_index = Some(val);
    //     self
    // }
    pub fn bind_to_device(mut self, val: String) -> Self {
        self.bind_to_device = Some(val);
        self
    }
    pub fn multicore(mut self, val: bool) -> Self {
        self.multicore = Some(val);
        self
    }
    pub fn num_event_loop(mut self, val: usize) -> Self {
        self.num_event_loop = Some(val);
        self
    }
    pub fn read_buffer_cap(mut self, val: usize) -> Self {
        self.read_buffer_cap = Some(val);
        self
    }
    pub fn write_buffer_cap(mut self, val: usize) -> Self {
        self.write_buffer_cap = Some(val);
        self
    }
    pub fn lock_os_thread(mut self, val: bool) -> Self {
        self.lock_os_thread = Some(val);
        self
    }
    pub fn ticker(mut self, val: bool) -> Self {
        self.ticker = Some(val);
        self
    }
    pub fn tcp_keepalive(mut self, val: Duration) -> Self {
        self.tcp_keepalive = Some(val);
        self
    }
    pub fn tcp_keepinterval(mut self, val: Duration) -> Self {
        self.tcp_keep_interval = Some(val);
        self
    }
    pub fn tcp_keepcount(mut self, val: u32) -> Self {
        self.tcp_keep_count = Some(val);
        self
    }
    pub fn tcp_no_delay(mut self, val: TcpSocketOpt) -> Self {
        self.tcp_no_delay = Some(val);
        self
    }
    pub fn socket_recv_buffer(mut self, val: usize) -> Self {
        self.socket_recv_buffer = Some(val);
        self
    }
    pub fn socket_send_buffer(mut self, val: usize) -> Self {
        self.socket_send_buffer = Some(val);
        self
    }
    pub fn edge_triggered_io_chunk(mut self, val: usize) -> Self {
        self.edge_triggered_io_chunk = Some(val);
        self
    }
    pub fn log_path(mut self, val: String) -> Self {
        self.log_path = Some(val);
        self
    }
    pub fn log_level(mut self, val: Level) -> Self {
        self.log_level = Some(val);
        self
    }
}

fn normalize_buffer_size(size: usize) -> usize {
    if size <= 0 {
        MAX_STREAM_BUFFER_CAP
    } else if size <= DEFAULT_BUFFER_SIZE {
        DEFAULT_BUFFER_SIZE
    } else {
        ceil_to_power_of_two(size)
    }
}

fn ceil_to_power_of_two(n: usize) -> usize {
    if n <= 2 { 2 } else { n.next_power_of_two() }
}


// 单元测试
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_buffer_cap_normalization() {
        let opts = Options::builder()
            .read_buffer_cap(0)
            .write_buffer_cap(0)
            .build();
        assert_eq!(opts.read_buffer_cap, MAX_STREAM_BUFFER_CAP);
        assert_eq!(opts.write_buffer_cap, MAX_STREAM_BUFFER_CAP);

        let opts = Options::builder()
            .read_buffer_cap(100)
            .write_buffer_cap(1024)
            .build();
        assert_eq!(opts.read_buffer_cap, 1024);
        assert_eq!(opts.write_buffer_cap, 1024);

        let opts = Options::builder()
            .read_buffer_cap(4097)
            .write_buffer_cap(8192)
            .build();
        assert_eq!(opts.read_buffer_cap, 8192);
        assert_eq!(opts.write_buffer_cap, 8192);
    }

    #[test]
    fn test_et_chunk_normalization() {
        let opts = Options::builder().build();
        assert_eq!(opts.edge_triggered_io_chunk, 1024 * 1024);

        let opts = Options::builder()
            .edge_triggered_io_chunk(100)
            .build();
        assert_eq!(opts.edge_triggered_io_chunk, 128);
    }

    #[test]
    fn test_normalize_udp() {
        let mut opts = Options::builder()
            .reuse_port(false)
            .build();

        opts.normalize(&["udp://127.0.0.1:8080"]).unwrap();

        assert_eq!(opts.reuse_port, true);
    }

    #[test]
    #[cfg(unix)]
    fn test_normalize_unix() {
        let mut opts = Options::builder()
            .reuse_port(true)
            .build();

        opts.normalize(&["unix:///tmp/test.sock"]).unwrap();

        assert_eq!(opts.reuse_port, false);
    }

    #[test]
    fn test_normalize_mixed() {
        let mut opts = Options::builder().reuse_port(false).build();
        // 如果是在 Unix 下，两个都能解析；非 Unix 下 "unix://" 解析会报错(InvalidInput)或Unsupport
        // 为了跨平台测试安全，这里我们只测 tcp+udp 混合，或者加 cfg
        #[cfg(unix)]
        {
            opts.normalize(&["tcp://:8080", "udp://:9090"]).unwrap();
            assert_eq!(opts.reuse_port, true);
        }
        #[cfg(not(unix))]
        {
            opts.normalize(&["tcp://:8080", "udp://:9090"]).unwrap();
            assert_eq!(opts.reuse_port, true);
        }
    }

    #[test]
    fn test_normalize_multicore_reuse_port_linux() {
        let mut opts = Options::builder().multicore(true).reuse_port(true).build();
        opts.normalize(&["tcp://:8080"]).unwrap();
        assert_eq!(opts.reuse_port, false);
    }

    #[test]
    fn test_ceil_to_power_of_two() {
        assert_eq!(ceil_to_power_of_two(0), 2);
        assert_eq!(ceil_to_power_of_two(1), 2);
        assert_eq!(ceil_to_power_of_two(2), 2);
        assert_eq!(ceil_to_power_of_two(3), 4);
        assert_eq!(ceil_to_power_of_two(100), 128);
        assert_eq!(ceil_to_power_of_two(1023), 1024);
        assert_eq!(ceil_to_power_of_two(1024), 1024);
        assert_eq!(ceil_to_power_of_two(1025), 2048);
    }
}