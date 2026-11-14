use std::io::{Error as IoError, ErrorKind, Read as _, Result as IoResult, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::helpers::HelperError;

use super::failed;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(10);

// one connection, one reply: the helper makes a single request, and the thread hands back its head
pub(super) fn serve(
    status: u16,
    body: &str,
) -> Result<(SocketAddr, JoinHandle<String>), Box<HelperError>> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| failed(&error))?;
    let address = listener.local_addr().map_err(|error| failed(&error))?;
    let reply = format!(
        "HTTP/1.1 {status} fixture\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();
    let request = thread::Builder::new()
        .spawn(move || {
            let Some(mut stream) = accept(&listener) else {
                return String::new();
            };
            let _read_timeout = stream.set_read_timeout(Some(READ_TIMEOUT));
            let head = read_head(&mut stream).unwrap_or_default();
            let _written = stream.write_all(&reply);
            head
        })
        .map_err(|error| failed(&error))?;
    Ok((address, request))
}

fn accept(listener: &TcpListener) -> Option<TcpStream> {
    if listener.set_nonblocking(true).is_err() {
        return None;
    }
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _peer)) => {
                if stream.set_nonblocking(false).is_err() {
                    return None;
                }
                return Some(stream);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if started.elapsed() >= CONNECT_TIMEOUT {
                    return None;
                }
                thread::park_timeout(RETRY);
            }
            Err(_refused) => return None,
        }
    }
}

fn read_head(stream: &mut TcpStream) -> IoResult<String> {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        received.extend_from_slice(&byte);
    }
    String::from_utf8(received).map_err(IoError::other)
}
