use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const MAX_REQUEST_BYTES: usize = 0x0010_0000;

pub type Reply = Arc<dyn Fn(&SeenRequest) -> (u16, String) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct SeenRequest {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

impl SeenRequest {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

pub struct Provider {
    pub base: String,
    pub seen: Arc<Mutex<Vec<SeenRequest>>>,
    tasks: JoinSet<()>,
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

pub fn transport() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

pub async fn serve_sequence(responses: Vec<(u16, String)>) -> Provider {
    let replies: Arc<Mutex<VecDeque<(u16, String)>>> = Arc::new(Mutex::new(responses.into()));
    let reply: Reply = Arc::new(move |_request: &SeenRequest| {
        replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or((500, "no reply left".to_owned()))
    });
    serve(reply).await
}

pub async fn serve(reply: Reply) -> Provider {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { accept(listener, reply, recorded).await });
    Provider { base, seen, tasks }
}

async fn accept(listener: TcpListener, reply: Reply, seen: Arc<Mutex<Vec<SeenRequest>>>) {
    loop {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let Ok(request) = read_request(&mut stream).await else {
            continue;
        };
        seen.lock().unwrap().push(request.clone());
        let (status, body) = reply(&request);
        write_response(&mut stream, status, &body).await;
    }
}

async fn read_request(stream: &mut TcpStream) -> io::Result<SeenRequest> {
    let mut buffer = Vec::new();
    let head_end = read_until_head_end(stream, &mut buffer).await?;
    let length = content_length(&buffer[..head_end]);
    read_body(stream, &mut buffer, head_end + length).await?;
    Ok(parse_request(&buffer, head_end, length))
}

async fn read_until_head_end(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> io::Result<usize> {
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            return Ok(position + 4);
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
    }
}

async fn read_body(stream: &mut TcpStream, buffer: &mut Vec<u8>, total: usize) -> io::Result<()> {
    let mut chunk = [0_u8; 4096];
    while buffer.len() < total {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    Ok(())
}

fn content_length(head: &[u8]) -> usize {
    from_utf8(head)
        .unwrap()
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0)
}

fn parse_request(buffer: &[u8], head_end: usize, length: usize) -> SeenRequest {
    let head = from_utf8(&buffer[..head_end]).unwrap();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap().split(' ');
    let method = request_line.next().unwrap().to_owned();
    let target = request_line.next().unwrap().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let body = from_utf8(&buffer[head_end..head_end + length])
        .unwrap()
        .to_owned();
    SeenRequest {
        method,
        target,
        headers,
        body,
    }
}

async fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} PROVIDER\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
}
