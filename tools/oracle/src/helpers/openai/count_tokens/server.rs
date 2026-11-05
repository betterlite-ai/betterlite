use std::collections::BTreeMap;
use std::io;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

pub(super) struct Reply<'text> {
    pub(super) status: u16,
    pub(super) body: &'text str,
}

pub(super) struct Recorded {
    path: String,
    host: String,
    content_type: Option<String>,
    authorization: Option<String>,
    body: Value,
}

impl Recorded {
    pub(super) fn view(&self, address: &str) -> Value {
        json!({
            "path": self.path,
            "content_type": self.content_type,
            "authorization": self.authorization,
            "body": self.body,
            "deployment_base": self.host == address,
        })
    }
}

async fn read_request(stream: &mut TcpStream) -> io::Result<Option<Recorded>> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut headers = BTreeMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .map_or(Ok(0), |text| text.parse())
        .map_err(io::Error::other)?;
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await?;
    Ok(Some(Recorded {
        path,
        host: headers.remove("host").unwrap_or_default(),
        content_type: headers.remove("content-type"),
        authorization: headers.remove("authorization"),
        body: serde_json::from_slice(&body)?,
    }))
}

// Answers each connection with the scripted reply and returns what it received. The caller ends the loop by connecting and closing without sending anything.
pub(super) async fn serve(listener: &TcpListener, reply: &Reply<'_>) -> io::Result<Vec<Recorded>> {
    let mut log = Vec::new();
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Some(recorded) = read_request(&mut stream).await? else {
            return Ok(log);
        };
        log.push(recorded);
        let head = format!(
            "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            reply.status,
            reply.body.len()
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(reply.body.as_bytes()).await?;
        stream.shutdown().await?;
    }
}
