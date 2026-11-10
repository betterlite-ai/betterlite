use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::str;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::failure;
use crate::helpers::HelperError;

#[derive(Clone, Debug)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct Captured {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

pub(super) struct Server {
    pub(super) base: String,
    captured: Arc<Mutex<Vec<Captured>>>,
    tasks: JoinSet<()>,
}

impl Server {
    pub(super) async fn start(replies: Vec<Reply>) -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| failure(&error))?;
        let address = listener.local_addr().map_err(|error| failure(&error))?;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, replies, Arc::clone(&captured)));
        Ok(Self {
            base: format!("http://{address}"),
            captured,
            tasks,
        })
    }

    pub(super) fn finish(mut self) -> Vec<Captured> {
        self.tasks.abort_all();
        self.captured
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

async fn serve(listener: TcpListener, replies: Vec<Reply>, captured: Arc<Mutex<Vec<Captured>>>) {
    let mut index = 0_usize;
    while let Ok((mut socket, _)) = listener.accept().await {
        let Some(reply) = replies.get(index).or_else(|| replies.last()) else {
            return;
        };
        index = index.saturating_add(1);
        let Ok(seen) = exchange(&mut socket, reply).await else {
            return;
        };
        let Ok(mut seen_list) = captured.lock() else {
            return;
        };
        seen_list.push(seen);
    }
}

async fn exchange(socket: &mut TcpStream, reply: &Reply) -> Result<Captured, String> {
    let mut buffer = Vec::new();
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let length = content_length(buffer.get(..end).unwrap_or_default())?;
            if buffer.len() >= end.saturating_add(4).saturating_add(length) {
                break;
            }
        }
        read_more(socket, &mut buffer).await?;
    }
    let end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "the request has no header end".to_owned())?;
    let length = content_length(buffer.get(..end).unwrap_or_default())?;
    let seen = parse(&buffer, end, length);
    write_response(socket, reply, &seen).await?;
    Ok(seen)
}

async fn read_more(socket: &mut TcpStream, buffer: &mut Vec<u8>) -> Result<(), String> {
    let mut chunk = [0_u8; 2048];
    let read = socket
        .read(&mut chunk)
        .await
        .map_err(|error| error.to_string())?;
    if read == 0 {
        return Err("the request ended before its headers".to_owned());
    }
    buffer.extend_from_slice(chunk.get(..read).unwrap_or_default());
    Ok(())
}

fn content_length(head: &[u8]) -> Result<usize, String> {
    let text = str::from_utf8(head).map_err(|error| error.to_string())?;
    Ok(text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0))
}

fn parse(buffer: &[u8], end: usize, length: usize) -> Captured {
    let head = str::from_utf8(buffer.get(..end).unwrap_or_default()).unwrap_or_default();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let body_start = end.saturating_add(4);
    let body = buffer
        .get(body_start..body_start.saturating_add(length))
        .unwrap_or_default()
        .to_vec();
    Captured {
        method,
        path,
        headers,
        body,
    }
}

async fn write_response(
    socket: &mut TcpStream,
    reply: &Reply,
    seen: &Captured,
) -> Result<(), String> {
    let host = seen
        .headers
        .get("host")
        .ok_or_else(|| "the request has no host header".to_owned())?;
    let origin = format!("http://{host}");
    let mut head = format!(
        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        let resolved = if value.starts_with('/') {
            format!("{origin}{value}")
        } else {
            value.clone()
        };
        write!(head, "{name}: {resolved}\r\n").map_err(|error| error.to_string())?;
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&reply.body);
    socket
        .write_all(&bytes)
        .await
        .map_err(|error| error.to_string())?;
    socket.shutdown().await.map_err(|error| error.to_string())
}
