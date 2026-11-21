use std::io;
use std::net::SocketAddr;

use tokio::net::TcpSocket;

use crate::loopback;

pub struct RefusedPort {
    pub address: SocketAddr,
    _socket: TcpSocket,
}

impl RefusedPort {
    pub fn bind() -> io::Result<Self> {
        let socket = loopback::bind()?;
        let address = socket.local_addr()?;
        Ok(Self {
            address,
            _socket: socket,
        })
    }
}

mod tests;
