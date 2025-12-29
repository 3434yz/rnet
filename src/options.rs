use crate::socket_addr::NetworkAddress;

use core_affinity::CoreId;

use std::collections::HashSet;
use std::io;
use std::time::Duration;

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
        } else {
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

pub fn get_core_ids(limit: Option<usize>) -> Vec<CoreId> {
    let all_cores = core_affinity::get_core_ids().unwrap_or_default();

    if limit.is_none() {
        return all_cores;
    }

    let target_count = limit.unwrap();
    if target_count == 0 {
        return all_cores;
    }

    if !cfg!(target_os = "linux") {
        return all_cores.into_iter().take(target_count).collect();
    }

    let mut selected_cores = Vec::with_capacity(target_count);
    let mut seen_physical_keys = HashSet::new();

    for core in all_cores {
        // 如果已经凑够了数量，立即停止
        if selected_cores.len() >= target_count {
            break;
        }

        // 获取物理拓扑指纹
        let key = get_linux_topology_key(core.id);

        if seen_physical_keys.insert(key) {
            selected_cores.push(core);
        }
    }

    selected_cores
}

#[derive(Debug, Hash, Eq, PartialEq)]
struct TopoKey {
    socket: String,
    core: String,
}

#[cfg(target_os = "linux")]
fn get_linux_topology_key(cpu_id: usize) -> TopoKey {
    use std::fs;
    use std::path::Path;

    let base = format!("/sys/devices/system/cpu/cpu{}/topology", cpu_id);
    let path = Path::new(&base);

    let socket = fs::read_to_string(path.join("physical_package_id"))
        .unwrap_or_else(|_| "0".to_string())
        .trim()
        .to_string();

    let core = fs::read_to_string(path.join("core_id"))
        .unwrap_or_else(|_| cpu_id.to_string())
        .trim()
        .to_string();

    TopoKey { socket, core }
}

#[cfg(not(target_os = "linux"))]
fn get_linux_topology_key(cpu_id: usize) -> TopoKey {
    TopoKey {
        socket: "0".to_string(),
        core: cpu_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limit_none_returns_all() {
        let all_system_cores = core_affinity::get_core_ids().unwrap();
        let result = get_core_ids(None);

        println!(
            "Test None: System has {} cores, got {}",
            all_system_cores.len(),
            result.len()
        );
        assert_eq!(result.len(), all_system_cores.len());
    }

    #[test]
    fn test_limit_some_filters_topology() {
        let target = 4;
        let result = get_core_ids(Some(target));

        println!("Test Some({}): Got {:?}", target, result);

        assert!(result.len() <= target);

        if cfg!(target_os = "linux") {
            let mut seen_phys = HashSet::new();
            for core in &result {
                let key = get_linux_topology_key(core.id);
                // 如果插入失败，说明结果里包含了同一个物理核的两个线程 -> 测试失败
                if !seen_phys.insert(key) {
                    panic!(
                        "Test Failed: Found duplicate physical core usage in result: {:?}",
                        core
                    );
                }
            }
            println!(
                "Test Some({}): Topology check passed. All cores are physically distinct.",
                target
            );
        }
    }

    #[test]
    fn test_limit_overflow() {
        let huge_limit = 1024;
        let result = get_core_ids(Some(huge_limit));

        println!(
            "Test Overflow: Requested {}, got {}",
            huge_limit,
            result.len()
        );
        assert!(result.len() > 0);
        assert!(result.len() <= core_affinity::get_core_ids().unwrap().len());
    }
}

#[cfg(test)]
mod test_option {
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

        let opts = Options::builder().edge_triggered_io_chunk(100).build();
        assert_eq!(opts.edge_triggered_io_chunk, 128);
    }

    #[test]
    fn test_normalize_udp() {
        let mut opts = Options::builder().reuse_port(false).build();

        opts.normalize(&["udp://127.0.0.1:8080"]).unwrap();

        assert_eq!(opts.reuse_port, true);
    }

    #[test]
    #[cfg(unix)]
    fn test_normalize_unix() {
        let mut opts = Options::builder().reuse_port(true).build();

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

    #[test]
    fn get_isolated_cores_from_affinity() {}
}
