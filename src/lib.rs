use ipnet::Ipv4Net;
use ipnet::Ipv6Net;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use std::fmt;
use std::io;
use std::net::IpAddr;
use std::net::Shutdown;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug)]
pub struct HostParseError;

#[derive(Debug)]
struct ResolveHostError;

/// Connects to a port and returns [true] if that was successful, [false] if not. The connection
/// is subsequently closed. Wraps an [io::Error], but does not propagate timeout errors, as
/// ports that do not answer within the timeout are considered offline.
///
/// ```
/// use std::net::SocketAddr;
/// use std::str::FromStr;
/// use std::time::Duration;
/// use is_online::is_port_online;
///
/// let addr = SocketAddr::from_str("8.8.8.8:53").unwrap();
/// let timeout = Duration::from_secs(3);
/// assert!(is_port_online(&addr, timeout).unwrap());
/// ```
///
/// This should only be used to check for open/closed ports. If for any reason you need to
/// re-use the established connection, use [TcpStream::connect_timeout] directly.
pub fn is_port_online(addr: &SocketAddr, timeout: Duration) -> Result<bool, std::io::Error> {
    let stream = TcpStream::connect_timeout(&addr, timeout);

    if let Err(e) = stream {
        if e.kind() == io::ErrorKind::TimedOut {
            return Ok(false);
        }

        return Err(e);
    }

    let stream = stream.unwrap();
    stream.shutdown(Shutdown::Both).ok();

    Ok(true)
}

/// A host that carries a name, and a list of addresses associated with that
/// name. The name can be a valid host name, FQDN, or IP address.
#[derive(Debug, Clone)]
pub struct Host {
    pub name: String,
    pub addresses: Vec<IpAddr>,
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

pub struct TcpPortCheck {
    port: u16,
    protocol: IpProtocol,
    timeout: Duration,
    strategy: CheckStrategy,
}

impl TcpPortCheck {
    /// The default port check
    pub fn default() -> Self {
        TcpPortCheck {
            port: 22,
            protocol: IpProtocol::Both,
            timeout: Duration::new(1, 0),
            strategy: CheckStrategy::Any,
        }
    }

    /// Change the port
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Change the protocol
    pub fn with_protocol(mut self, protocol: IpProtocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Change the timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Change the strategy
    pub fn with_strategy(mut self, strategy: CheckStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Returns true if the given host is online
    pub fn is_online(&self, host: &Host) -> bool {
        let addrs: Vec<&IpAddr> = match self.protocol {
            IpProtocol::Both => host.addresses.iter().collect(),
            IpProtocol::V4 => host.addresses.iter().filter(|a| a.is_ipv4()).collect(),
            IpProtocol::V6 => host.addresses.iter().filter(|a| a.is_ipv6()).collect(),
        };

        if addrs.is_empty() {
            return false;
        }

        let iter = addrs.par_iter();

        match self.strategy {
            CheckStrategy::Any => iter.any(|addr| self.is_open_port(addr)),
            CheckStrategy::All => iter.all(|addr| self.is_open_port(addr)),
        }
    }

    /// Creates a new vector from the given references, containing only online hosts.
    pub fn collect_online(&self, hosts: &[Host]) -> Vec<Host> {
        hosts
            .par_iter()
            .filter(|host| self.is_online(host))
            .cloned()
            .collect()
    }

    /// Check if we can create a connection to the given socket
    fn is_open_port(&self, addr: &IpAddr) -> bool {
        let socket = SocketAddr::new(*addr, self.port);

        if let Ok(stream) = TcpStream::connect_timeout(&socket, self.timeout) {
            let _ = stream.shutdown(Shutdown::Both);
            return true;
        }

        false
    }
}

/// Defines how online checks are executed
#[derive(Debug)]
pub enum IpProtocol {
    /// Connect using IPv4 or IPv6
    Both,

    /// Connect using any IPv4 address
    V4,

    /// Connect using any IPv6 address
    V6,
}

/// Defines what online check strategy is used
#[derive(Debug)]
pub enum CheckStrategy {
    /// Consider a host online if any known address is online
    Any,

    /// Consider a host online if all known addresses are online
    All,
}

impl FromStr for Host {
    type Err = HostParseError;

    fn from_str(host: &str) -> Result<Self, Self::Err> {
        // If the host is an address, parse it
        if let Ok(address) = IpAddr::from_str(host) {
            return Ok(Host {
                name: String::from(host),
                addresses: vec![address],
            });
        };

        // If not an address, resolve it
        if let Ok(addresses) = resolve_hostname(host) {
            return Ok(Host {
                name: String::from(host),
                addresses,
            });
        };

        Err(HostParseError)
    }
}

/// Takes a list of host-names and yields all hosts that can be resolved
pub fn resolve_hosts(hosts: &[String]) -> Vec<Host> {
    hosts
        .par_iter()
        .filter_map(|host| {
            if let Ok(host) = Host::from_str(host) {
                Some(host)
            } else {
                None
            }
        })
        .collect()
}

/// Takes a list of host-names and expands the ones that are IP subnets into IP addresses, excluding
/// broadcast addresses.
pub fn expand_subnets(hosts: &[String]) -> Vec<String> {
    let mut expanded: Vec<String> = Vec::with_capacity(hosts.len());

    for host in hosts {
        if let Ok(net) = Ipv4Net::from_str(host) {
            for host in net.hosts() {
                if host.is_unspecified() || host.is_broadcast() {
                    continue;
                }

                expanded.push(host.to_string());
            }

            continue;
        }

        if let Ok(net) = Ipv6Net::from_str(host) {
            for host in net.hosts() {
                if host.is_unspecified() {
                    continue;
                }

                expanded.push(host.to_string());
            }

            continue;
        }

        expanded.push(host.clone());
    }

    expanded
}

/// Resolve the given hostname and return a vector of IP addresses. If given
/// an IP address, it will be wrapped in a vector sans lookup.
fn resolve_hostname(name: &str) -> Result<Vec<IpAddr>, ResolveHostError> {
    // If the hostname is an IP address, return it
    if let Ok(address) = IpAddr::from_str(name) {
        return Ok(vec![address]);
    }

    // To parse a socket, we need a port (even though we don't care about it)
    let name = format!("{}:0", name);

    // Return all resolved IP addresses
    if let Ok(addresses) = name.to_socket_addrs() {
        return Ok(addresses.map(|socket| socket.ip()).collect());
    };

    Err(ResolveHostError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::net::Ipv6Addr;

    #[test]
    fn test_resolve_hostname() {
        let address = resolve_hostname("localhost").unwrap();

        assert!(
            address
                == vec![
                    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                    IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)),
                ]
                || address
                    == vec![
                        IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)),
                        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                    ]
        );
    }

    #[test]
    fn test_parse_address() {
        let host = Host::from_str("127.0.0.1").unwrap();

        assert_eq!(&host.name, "127.0.0.1");
        assert_eq!(
            host.addresses,
            vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),]
        );
    }

    #[test]
    fn test_parse_name() {
        let host = Host::from_str("localhost").unwrap();

        assert_eq!(&host.name, "localhost");
        assert!(
            host.addresses
                == vec![
                    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                    IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)),
                ]
                || host.addresses
                    == vec![
                        IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)),
                        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                    ]
        );
    }

    #[test]
    fn is_tcp_port_online() {
        let host = Host::from_str("google.com").unwrap();

        // Check if the TCP port is online (any protocol)
        assert!(TcpPortCheck::default()
            .with_port(80)
            .with_protocol(IpProtocol::Both)
            .is_online(&host));

        // Check if the TCP port is online (IPv4)
        assert!(TcpPortCheck::default()
            .with_port(80)
            .with_protocol(IpProtocol::V4)
            .is_online(&host));

        // Check if the TCP port is online (IPv6)
        assert!(TcpPortCheck::default()
            .with_port(80)
            .with_protocol(IpProtocol::V6)
            .is_online(&host));

        // Querying an IPv6 only host with IPv4 returns false
        let host = Host::from_str("ipv6.google.com").unwrap();

        assert!(!TcpPortCheck::default()
            .with_port(80)
            .with_protocol(IpProtocol::V4)
            .is_online(&host));
    }
}
