use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::failure;

pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

pub(super) struct Captured {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

pub(super) struct Server {
    pub(super) base: String,
    task: JoinHandle<Result<Vec<Captured>, String>>,
}

impl Server {
    pub(super) async fn start(
        replies: Vec<Reply>,
    ) -> Result<Self, Box<crate::helpers::HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| failure(&error))?;
        let address = listener.local_addr().map_err(|error| failure(&error))?;
        let task = tokio::spawn(serve(listener, replies));
        Ok(Self {
            base: format!("http://{address}"),
            task,
        })
    }

    pub(super) async fn finish(self) -> Result<Vec<Captured>, Box<crate::helpers::HelperError>> {
        let joined = timeout(Duration::from_secs(10), self.task)
            .await
            .map_err(|_elapsed| failure("the provider saw fewer requests than expected"))?;
        let captured = joined.map_err(|error| failure(&error))?.map_err(failure)?;
        Ok(captured)
    }
}

async fn serve(listener: TcpListener, replies: Vec<Reply>) -> Result<Vec<Captured>, String> {
    let mut captured = Vec::with_capacity(replies.len());
    for reply in replies {
        let (mut socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let request = read_request(&mut socket).await?;
        write_response(&mut socket, &reply).await?;
        captured.push(request);
    }
    Ok(captured)
}

async fn read_request(socket: &mut TcpStream) -> Result<Captured, String> {
    let mut buffer = read_head(socket).await?;
    let head_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "the request has no header end".to_owned())?;
    let head = str::from_utf8(buffer.get(..head_end).unwrap_or_default())
        .map_err(|error| error.to_string())?;
    let (method, path) = request_target(head)?;
    let headers = request_headers(head)?;
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = head_end.saturating_add(4);
    while buffer.len().saturating_sub(body_start) < length {
        let mut chunk = [0_u8; 4096];
        let read = socket
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(chunk.get(..read).unwrap_or_default());
    }
    let body = buffer
        .get(body_start..body_start.saturating_add(length))
        .unwrap_or_default()
        .to_vec();
    Ok(Captured {
        method,
        path,
        headers,
        body,
    })
}

async fn read_head(socket: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    loop {
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(buffer);
        }
        let mut chunk = [0_u8; 4096];
        let read = socket
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("the request ended before its headers".to_owned());
        }
        buffer.extend_from_slice(chunk.get(..read).unwrap_or_default());
    }
}

fn request_target(head: &str) -> Result<(String, String), String> {
    let line = head
        .split("\r\n")
        .next()
        .ok_or_else(|| "the request has no request line".to_owned())?;
    let mut parts = line.split(' ');
    let method = parts
        .next()
        .ok_or_else(|| "the request line has no method".to_owned())?;
    let path = parts
        .next()
        .ok_or_else(|| "the request line has no target".to_owned())?;
    Ok((method.to_owned(), path.to_owned()))
}

fn request_headers(head: &str) -> Result<BTreeMap<String, String>, String> {
    let mut headers = BTreeMap::new();
    for line in head.split("\r\n").skip(1) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| format!("the header line {line:?} has no colon"))?;
        let _previous = headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    Ok(headers)
}

async fn write_response(socket: &mut TcpStream, reply: &Reply) -> Result<(), String> {
    let head = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.body.len()
    );
    socket
        .write_all(head.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    socket
        .write_all(&reply.body)
        .await
        .map_err(|error| error.to_string())?;
    socket.shutdown().await.map_err(|error| error.to_string())
}
