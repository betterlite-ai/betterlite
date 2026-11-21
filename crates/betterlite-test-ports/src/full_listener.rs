use std::io;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use tokio::net::{TcpListener, TcpSocket};

use crate::loopback;

const BACKLOG: u32 = 1;
const ATTEMPT_TIMEOUT: Duration = Duration::from_millis(100);
const FILL_ATTEMPTS: u8 = 4;

pub struct FullListener {
    pub address: SocketAddr,
    _listener: TcpListener,
    _fillers: Vec<TcpStream>,
}

impl FullListener {
    // listen registers the socket with the reactor, so call this from inside a tokio test
    pub fn bind() -> io::Result<Self> {
        let socket = loopback::bind()?;
        let listener = socket.listen(BACKLOG)?;
        let address = listener.local_addr()?;
        let fillers = (0_u8..FILL_ATTEMPTS)
            .filter_map(|_| TcpStream::connect_timeout(&address, ATTEMPT_TIMEOUT).ok())
            .collect();
        Ok(Self {
            address,
            _listener: listener,
            _fillers: fillers,
        })
    }
}

mod tests;
