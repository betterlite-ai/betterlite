use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};

use socket2::{Domain, Protocol, SockAddr, Socket, Type};

pub trait PortProbe {
    fn is_available(&self, port: u16) -> bool;
}

pub struct LoopbackPortProbe;

impl PortProbe for LoopbackPortProbe {
    fn is_available(&self, port: u16) -> bool {
        bind_loopback(port).is_ok()
    }
}

// LiteLLM sets SO_REUSEADDR before bind, so the option goes on the socket ahead of the address
pub fn bind_loopback(port: u16) -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    socket.bind(&SockAddr::from(address))?;
    Ok(TcpListener::from(socket))
}
