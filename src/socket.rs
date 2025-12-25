use mio::net::{TcpStream, UnixStream};
use mio::{Interest, Registry, Token, event::Source};
use std::io::{self, Read, Write};

#[derive(Debug)]
pub enum Socket {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Socket {
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_nodelay(nodelay),
            #[cfg(unix)]
            Self::Unix(_) => Ok(()),
        }
    }
}

impl Source for Socket {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match self {
            Self::Tcp(s) => registry.register(s, token, interests),
            #[cfg(unix)]
            Self::Unix(s) => registry.register(s, token, interests),
        }
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match self {
            Self::Tcp(s) => registry.reregister(s, token, interests),
            #[cfg(unix)]
            Self::Unix(s) => registry.reregister(s, token, interests),
        }
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        match self {
            Self::Tcp(s) => registry.deregister(s),
            #[cfg(unix)]
            Self::Unix(s) => registry.deregister(s),
        }
    }
}

impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Self::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Self::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
        }
    }
}
