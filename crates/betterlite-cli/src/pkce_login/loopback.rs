use std::io::{Error as IoError, ErrorKind, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

// Matches LoopbackServer.connection_timeout_seconds in LiteLLM: a connection that opens and sends nothing is dropped after this long so it cannot hold the wait past its deadline.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

pub trait LoopbackListener {
    type Connection: LoopbackConnection;

    fn local_addr(&self) -> Result<SocketAddr, IoError>;

    // Returns Ok(None) when no connection is pending right now (a non-blocking accept would have blocked); the caller polls again rather than treating that as an error.
    fn accept(&self) -> Result<Option<Self::Connection>, IoError>;
}

pub trait LoopbackConnection {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, IoError>;

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), IoError>;
}

pub struct TcpLoopbackListener {
    listener: TcpListener,
}

impl TcpLoopbackListener {
    pub(super) fn bind() -> Result<Self, IoError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener })
    }
}

impl LoopbackListener for TcpLoopbackListener {
    type Connection = TcpLoopbackConnection;

    fn local_addr(&self) -> Result<SocketAddr, IoError> {
        self.listener.local_addr()
    }

    fn accept(&self) -> Result<Option<Self::Connection>, IoError> {
        match self.listener.accept() {
            Ok((stream, _peer)) => {
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
                Ok(Some(TcpLoopbackConnection { stream }))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

pub struct TcpLoopbackConnection {
    stream: TcpStream,
}

impl LoopbackConnection for TcpLoopbackConnection {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, IoError> {
        self.stream.read(buffer)
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        self.stream.write_all(bytes)
    }
}
