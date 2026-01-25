use mio::event::Source;
use mio::net::{TcpListener, UnixListener};
use mio::{Interest, Registry, Token};
use std::io::{self};
use std::sync::Arc;

#[cfg(unix)]
use std::path::Path;

use crate::options::Options;
use crate::socket::Socket;
use crate::socket_addr::NetworkAddress;

#[derive(Debug)]
pub struct Listener {
    pub(crate) inner: ListenerInner,
    pub(crate) addr: NetworkAddress,
    pub(crate) options: Arc<Options>,
}

#[derive(Debug)]
pub enum ListenerInner {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

impl ListenerInner {
    pub fn local_addr(&self) -> io::Result<NetworkAddress> {
        match self {
            Self::Tcp(l) => l.local_addr().map(NetworkAddress::Tcp),
            #[cfg(unix)]
            Self::Unix(l) => l.local_addr().and_then(NetworkAddress::try_from),
        }
    }
}

impl Listener {
    pub fn bind(addr: NetworkAddress, options: Arc<Options>) -> io::Result<Self> {
        let backlog = get_max_listen_backlog();

        match addr {
            NetworkAddress::Tcp(socket_addr) => {
                let listener = create_mio_tcp_listener(socket_addr, &options, backlog)?;

                // 2. 获取实际端口信息
                let local_addr = listener.local_addr()?;
                Ok(Listener {
                    inner: ListenerInner::Tcp(listener),
                    addr: NetworkAddress::Tcp(local_addr),
                    options,
                })
            }

            #[cfg(unix)]
            NetworkAddress::Unix(path) => {
                let listener = create_mio_unix_listener(&path, &options, backlog)?;

                Ok(Listener {
                    inner: ListenerInner::Unix(listener),
                    addr: NetworkAddress::Unix(path),
                    options,
                })
            }

            // NetworkAddress::Udp(_) => Err(Error::new(
            //     ErrorKind::InvalidInput,
            //     "Listener does not support UDP",
            // )),
            #[cfg(not(unix))]
            NetworkAddress::Unix(_) => Err(Error::new(
                ErrorKind::Unsupported,
                "Unix sockets are not supported on this platform",
            )),
        }
    }

    pub fn local_addr(&self) -> io::Result<NetworkAddress> {
        Ok(self.addr.clone())
    }

    pub fn network(&self) -> &'static str {
        match &self.addr {
            NetworkAddress::Tcp(addr) => {
                if addr.is_ipv4() {
                    "tcp4"
                } else {
                    "tcp6"
                }
            }
            #[cfg(unix)]
            NetworkAddress::Unix(_) => "unix",
            // NetworkAddress::Udp(addr) => {
            //     if addr.is_ipv4() {
            //         "udp4"
            //     } else {
            //         "udp6"
            //     }
            // }
        }
    }

    pub fn accept(&self) -> io::Result<(Socket, NetworkAddress)> {
        match &self.inner {
            ListenerInner::Tcp(l) => {
                let (stream, addr) = l.accept()?;
                Ok((Socket::Tcp(stream), NetworkAddress::Tcp(addr)))
            }
            #[cfg(unix)]
            ListenerInner::Unix(l) => {
                let (stream, addr) = l.accept()?;
                let net_addr = if let Some(path) = addr.as_pathname() {
                    NetworkAddress::Unix(path.to_path_buf())
                } else {
                    // 对于匿名 socket，我们构造一个特殊的 PathBuf 或者是 Log
                    NetworkAddress::Unix(std::path::PathBuf::from("unnamed_unix_socket"))
                };
                Ok((Socket::Unix(stream), net_addr))
            }
        }
    }
}

impl Source for Listener {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match &mut self.inner {
            ListenerInner::Tcp(l) => registry.register(l, token, interests),
            #[cfg(unix)]
            ListenerInner::Unix(l) => registry.register(l, token, interests),
        }
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match &mut self.inner {
            ListenerInner::Tcp(l) => registry.reregister(l, token, interests),
            #[cfg(unix)]
            ListenerInner::Unix(l) => registry.reregister(l, token, interests),
        }
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        match &mut self.inner {
            ListenerInner::Tcp(l) => registry.deregister(l),
            #[cfg(unix)]
            ListenerInner::Unix(l) => registry.deregister(l),
        }
    }
}

fn create_mio_tcp_listener(
    addr: std::net::SocketAddr,
    options: &Options,
    backlog: i32,
) -> io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};

    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;

    configure_socket(&socket, options)?;

    socket.bind(&addr.into())?;
    socket.listen(backlog)?;

    socket.set_nonblocking(true)?;
    if let crate::options::TcpSocketOpt::NoDelay = options.tcp_no_delay {
        socket.set_tcp_nodelay(true)?;
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly"))]
    if let Some(keepalive_time) = options.tcp_keepalive {
        socket.set_keepalive(true)?;
        let interval = options.tcp_keep_interval.unwrap_or(keepalive_time / 5);
        let retries = options.tcp_keep_count.unwrap_or(5);

        let mut ka_params = socket2::TcpKeepalive::new()
            .with_time(keepalive_time)
            .with_interval(interval);
        ka_params = ka_params.with_retries(retries);
        socket.set_tcp_keepalive(&ka_params)?;
    }

    let std_listener: std::net::TcpListener = socket.into();
    let mio_listener = TcpListener::from_std(std_listener);

    Ok(mio_listener)
}

#[cfg(unix)]
fn create_mio_unix_listener(
    path: &Path,
    options: &Options,
    backlog: i32,
) -> io::Result<UnixListener> {
    use socket2::{Domain, SockAddr, Socket, Type};

    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    configure_socket(&socket, options)?;

    if path.exists() {
        let _ = std::fs::remove_file(path);
    }

    let addr = SockAddr::unix(path)?;
    socket.bind(&addr)?;
    socket.listen(backlog)?;

    // 转换链
    socket.set_nonblocking(true)?;
    let std_listener: std::os::unix::net::UnixListener = socket.into();
    let mio_listener = UnixListener::from_std(std_listener);

    Ok(mio_listener)
}

fn configure_socket(socket: &socket2::Socket, options: &Options) -> io::Result<()> {
    if options.reuse_addr {
        socket.set_reuse_address(true)?;
    }

    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    if options.reuse_port {
        socket.set_reuse_port(true)?;
    }

    if options.socket_recv_buffer > 0 {
        socket.set_recv_buffer_size(options.socket_recv_buffer)?;
    }

    if options.socket_send_buffer > 0 {
        socket.set_send_buffer_size(options.socket_send_buffer)?;
    }

    #[cfg(linux)]
    if !options.bind_to_device.is_empty() {
        let device = options.bind_to_device.as_bytes();
        socket.bind_device(Some(device))?;
    }
    Ok(())
}

fn get_max_listen_backlog() -> i32 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/sys/net/core/somaxconn") {
            if let Ok(n) = s.trim().parse::<i32>() {
                return if n > 65535 { 65535 } else { n };
            }
        }
    }
    128
}
