use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Protocol {
    Tcp,
    // Udp,
    #[cfg(unix)]
    Unix,
}

#[derive(Debug, Clone)]
pub enum NetworkAddress {
    Tcp(SocketAddr),
    // Udp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

impl NetworkAddress {
    pub fn protocol(&self) -> Protocol {
        match self {
            Self::Tcp(_) => Protocol::Tcp,
            // Self::Udp(_) => Protocol::Udp,
            #[cfg(unix)]
            Self::Unix(_) => Protocol::Unix,
        }
    }
}

impl std::fmt::Display for NetworkAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(addr) => write!(f, "tcp://{}", addr),
            // Self::Udp(addr) => write!(f, "udp://{}", addr),
            #[cfg(unix)]
            Self::Unix(path) => {
                write!(f, "unix://{}", path.display())
            }
        }
    }
}

impl FromStr for NetworkAddress {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (proto_str, body) = s
            .split_once("://")
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "missing '://' separator"))?;

        if body.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "address body cannot be empty",
            ));
        }

        match proto_str {
            "tcp" | "tcp4" | "tcp6" => {
                let addr_str = if body.starts_with(':') {
                    format!("0.0.0.0{}", body)
                } else {
                    body.to_string()
                };

                let addr: SocketAddr = addr_str.parse().map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        format!("invalid tcp address: {}", e),
                    )
                })?;
                Ok(NetworkAddress::Tcp(addr))
            }
            // "udp" | "udp4" | "udp6" => {
            //     let addr_str = if body.starts_with(':') {
            //         format!("0.0.0.0{}", body)
            //     } else {
            //         body.to_string()
            //     };
            //
            //     let addr: SocketAddr = addr_str.parse().map_err(|e| {
            //         Error::new(
            //             ErrorKind::InvalidInput,
            //             format!("invalid udp address: {}", e),
            //         )
            //     })?;
            //     Ok(NetworkAddress::Udp(addr))
            // }
            #[cfg(unix)]
            "unix" => Ok(NetworkAddress::Unix(PathBuf::from(body))),

            _ => Err(Error::new(
                ErrorKind::InvalidInput,
                format!("unsupported protocol: {}", proto_str),
            )),
        }
    }
}

#[cfg(unix)]
use std::convert::TryFrom;
#[cfg(unix)]
impl TryFrom<std::os::unix::net::SocketAddr> for NetworkAddress {
    type Error = Error;

    fn try_from(sys_addr: std::os::unix::net::SocketAddr) -> Result<Self, Self::Error> {
        if let Some(path) = sys_addr.as_pathname() {
            Ok(NetworkAddress::Unix(path.to_path_buf()))
        } else {
            Err(Error::new(
                ErrorKind::AddrNotAvailable,
                "unix socket is not bound to a filesystem path (abstract or unnamed)",
            ))
        }
    }
}

impl TryFrom<SocketAddr> for NetworkAddress {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self, Self::Error> {
        Ok(NetworkAddress::Tcp(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_parse_valid_tcp() {
        // 1. 正常 TCP (v4)
        let s = "tcp://127.0.0.1:8080";
        let addr: NetworkAddress = s.parse().expect("should parse valid tcp");
        match addr {
            NetworkAddress::Tcp(sa) => {
                assert_eq!(sa.ip(), IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
                assert_eq!(sa.port(), 8080);
                // 验证 Display 实现是否正确
                assert_eq!(addr.to_string(), "tcp://127.0.0.1:8080");
            }
            _ => panic!("expected Tcp variant"),
        }

        let s = "tcp4://0.0.0.0:9000";
        let addr: NetworkAddress = s.parse().expect("should parse tcp4");
        if let NetworkAddress::Tcp(sa) = addr {
            assert!(sa.ip().is_unspecified()); // 0.0.0.0
            assert_eq!(sa.port(), 9000);
        } else {
            panic!("expected Tcp variant for tcp4 scheme");
        }

        let s = "tcp6://[::1]:9000";
        let addr: NetworkAddress = s.parse().expect("should parse tcp6");
        if let NetworkAddress::Tcp(sa) = addr {
            assert_eq!(sa.ip(), IpAddr::V6(Ipv6Addr::LOCALHOST));
            assert_eq!(sa.port(), 9000);
            assert_eq!(addr.to_string(), "tcp://[::1]:9000");
        } else {
            panic!("expected Tcp variant for tcp6 scheme");
        }

        let s = "tcp://:8080";
        let addr: NetworkAddress = s.parse().expect("should parse implicit host");
        if let NetworkAddress::Tcp(sa) = addr {
            assert_eq!(sa.ip(), IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
            assert_eq!(sa.port(), 8080);
            assert_eq!(addr.to_string(), "tcp://0.0.0.0:8080");
        } else {
            panic!("expected Tcp variant");
        }
    }

    // #[test]
    // fn test_parse_valid_udp() {
    //     // 正常 UDP
    //     let s = "udp://127.0.0.1:9991";
    //     let addr: NetworkAddress = s.parse().expect("should parse udp");
    //     match addr {
    //         NetworkAddress::Udp(sa) => {
    //             assert_eq!(sa.ip(), IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    //             assert_eq!(sa.port(), 9991);
    //             assert_eq!(addr.to_string(), "udp://127.0.0.1:9991");
    //         }
    //         _ => panic!("expected Udp variant"),
    //     }
    // }
    // #[test]
    // fn test_parse_udp_scope_id() {
    //     let s = "udp://[ff02::3%lo0]:9991";
    //     if let Ok(addr) = s.parse::<NetworkAddress>() {
    //         if let NetworkAddress::Udp(sa) = addr {
    //             assert_eq!(sa.port(), 9991);
    //             assert!(sa.is_ipv6());
    //         } else {
    //             panic!("expected Udp variant");
    //         }
    //     }
    // }
    #[test]
    #[cfg(unix)]
    fn test_parse_valid_unix() {
        // 对应旧测试：unix path
        let s = "unix:///tmp/rnet.sock";
        let addr: NetworkAddress = s.parse().expect("should parse unix");
        match addr {
            NetworkAddress::Unix(ref path) => {
                assert_eq!(path.to_str().unwrap(), "/tmp/rnet.sock");
                assert_eq!(addr.to_string(), "unix:///tmp/rnet.sock");
            }
            _ => panic!("expected Unix variant"),
        }
    }

    #[test]
    fn test_parse_bad() {
        // 1. 错误的协议
        let err = "tulip://howdy".parse::<NetworkAddress>();
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);

        // 2. 缺少分隔符
        let err = "howdy".parse::<NetworkAddress>();
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().to_string(), "missing '://' separator");

        // 3. 缺少地址体
        let err = "tcp://".parse::<NetworkAddress>();
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().to_string(), "address body cannot be empty");

        let err = "unix://".parse::<NetworkAddress>();
        assert!(err.is_err());

        // 4. 旧测试用例 ":foo"
        // 同样会被判定为缺少分隔符 "://"，因为 split_once 找不到
        let err = ":foo".parse::<NetworkAddress>();
        assert!(err.is_err());
    }

    #[test]
    fn test_edge_cases() {
        // 1. 大写协议 (Rust match 区分大小写) -> 应当报错
        let err = "TCP://127.0.0.1:80".parse::<NetworkAddress>();
        assert!(err.is_err());

        // 2. 包含路径 (对于 TCP/UDP)
        // 标准库 SocketAddr parse 不支持 "/foo"，所以这里会报错，符合预期
        let err = "tcp://127.0.0.1:8080/foo".parse::<NetworkAddress>();
        assert!(err.is_err());
        // 错误信息应该来自 parse::<SocketAddr>
        assert!(err.unwrap_err().to_string().contains("invalid tcp address"));
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

// #[test]
// fn test_parse_proto_addr_valid() {
//     // 正常 TCP
//     assert_eq!(
//         parse_proto_addr("tcp://127.0.0.1:8080").unwrap(),
//         ("tcp", "127.0.0.1:8080")
//     );
//     assert_eq!(
//         parse_proto_addr("tcp4://0.0.0.0:9000").unwrap(),
//         ("tcp4", "0.0.0.0:9000")
//     );
//     assert_eq!(
//         parse_proto_addr("tcp6://[::1]:9000").unwrap(),
//         ("tcp6", "[::1]:9000")
//     );
//
//     // [新增] 允许省略 IP 的写法 (对应上一轮的修改)
//     assert_eq!(
//         parse_proto_addr("tcp://:8080").unwrap(),
//         ("tcp", ":8080")
//     );
//
//     // 正常 UDP
//     assert_eq!(
//         parse_proto_addr("udp://127.0.0.1:9991").unwrap(),
//         ("udp", "127.0.0.1:9991")
//     );
//
//     // 正常 Unix
//     assert_eq!(
//         parse_proto_addr("unix:///tmp/rnet.sock").unwrap(),
//         ("unix", "/tmp/rnet.sock")
//     );
// }
//
// #[test]
// fn parse_valid_udp_with_scope_id() {
//     // IPv6 Link-Local 地址通常包含 %scope_id (如 %eth0)
//     let res = parse_proto_addr("udp://[ff02::3%lo0]:9991").unwrap();
//     assert_eq!(res, ("udp", "[ff02::3%lo0]:9991"));
// }
//
// #[test]
// fn test_parse_proto_addr_bad() {
//     // 错误的协议
//     let err = parse_proto_addr("tulip://howdy");
//     assert!(err.is_err());
//     assert_eq!(err.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
//
//     // 缺少分隔符
//     let err = parse_proto_addr("howdy");
//     assert!(err.is_err());
//
//     // 缺少地址体
//     let err = parse_proto_addr("tcp://");
//     assert!(err.is_err());
//
//     let err = parse_proto_addr("unix://");
//     assert!(err.is_err());
//
//     // [删除] tcp://:8080 以前是错误，现在是正确，所以已从此处移除
//
//     // 原始测试用例 ":foo" 在没有协议头的情况下会被判定为缺少分隔符 "://"
//     let err = parse_proto_addr(":foo");
//     assert!(err.is_err());
// }
//
// #[test]
// fn test_parse_proto_addr_edge_cases() {
//     // 大写协议 (Rust match 区分大小写) -> 应当报错
//     assert!(parse_proto_addr("TCP://127.0.0.1:80").is_err());
//
//     // 包含路径 -> 应当报错
//     let res = parse_proto_addr("tcp://127.0.0.1:8080/foo");
//     assert!(res.is_err());
// }
// }
