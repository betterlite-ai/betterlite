use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::helpers::HelperError;

use super::failure;

pub(super) struct Captured {
    pub(super) path: String,
    pub(super) query: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: serde_json::Value,
}

pub(super) struct Server {
    pub(super) base: String,
    task: JoinHandle<Result<Captured, String>>,
}

impl Server {
    pub(super) async fn start(status: u16, answer: Vec<u8>) -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| failure("OSError", &error))?;
        let address = listener
            .local_addr()
            .map_err(|error| failure("OSError", &error))?;
        let task = tokio::spawn(serve(listener, status, answer));
        Ok(Self {
            base: format!("http://{address}"),
            task,
        })
    }

    // a completion that fails before sending never opens the connection, so no request is reported
    pub(super) async fn finish(self) -> Result<Option<Captured>, Box<HelperError>> {
        match timeout(Duration::from_secs(5), self.task).await {
            Ok(joined) => {
                let captured = joined
                    .map_err(|error| failure("RuntimeError", &error))?
                    .map_err(|message| failure("ValueError", &message))?;
                Ok(Some(captured))
            }
            Err(_elapsed) => Ok(None),
        }
    }
}

async fn serve(listener: TcpListener, status: u16, answer: Vec<u8>) -> Result<Captured, String> {
    let (mut stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
    let captured = read_request(&mut stream).await?;
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        answer.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(&answer)
        .await
        .map_err(|error| error.to_string())?;
    stream.shutdown().await.map_err(|error| error.to_string())?;
    Ok(captured)
}

async fn read_request(stream: &mut TcpStream) -> Result<Captured, String> {
    let head = read_head(stream).await?;
    let mut lines = head.split("\r\n");
    let target = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .ok_or_else(|| String::from("the request line has no target"))?;
    let headers = request_headers(lines);
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut raw = vec![0_u8; length];
    stream
        .read_exact(&mut raw)
        .await
        .map_err(|error| error.to_string())?;
    let body = serde_json::from_slice(&raw).map_err(|error| error.to_string())?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Ok(Captured {
        path: path.to_owned(),
        query: query.to_owned(),
        headers,
        body,
    })
}

async fn read_head(stream: &mut TcpStream) -> Result<String, String> {
    let mut raw = Vec::new();
    loop {
        if raw.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        let mut byte = [0_u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|error| error.to_string())?;
        raw.push(byte[0]);
    }
    String::from_utf8(raw).map_err(|error| error.to_string())
}

fn request_headers<'line, Lines>(lines: Lines) -> BTreeMap<String, String>
where
    Lines: Iterator<Item = &'line str>,
{
    let mut headers = BTreeMap::new();
    for (name, value) in lines.filter_map(|line| line.split_once(':')) {
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    headers
}
