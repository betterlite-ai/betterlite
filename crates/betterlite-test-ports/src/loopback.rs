use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::net::TcpSocket;

// SO_REUSEADDR stays off: with it, a second bind to the port would succeed and the port would leak
pub(crate) fn bind() -> io::Result<TcpSocket> {
    let socket = TcpSocket::new_v4()?;
    socket.bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
    Ok(socket)
}
