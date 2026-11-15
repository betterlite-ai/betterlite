use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use http::header::CONTENT_LENGTH;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use url::Url;

use super::RecordedRequest;

pub(super) struct RawResponse {
    status: u16,
    content_type: &'static str,
    body: &'static [u8],
    declared: usize,
}

// a content-length past the body, so the client's body read fails after the head arrives
pub(super) const MISSING_BYTES: usize = 4096;

impl RawResponse {
    pub(super) const fn new(
        status: u16,
        content_type: &'static str,
        body: &'static [u8],
        missing: usize,
    ) -> Self {
        Self {
            status,
            content_type,
            body,
            declared: body.len().saturating_add(missing),
        }
    }
}

pub(super) struct RawTestServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    _acceptor: JoinSet<()>,
}

pub(super) type RawReply = Arc<dyn Fn(&RecordedRequest) -> RawResponse + Send + Sync>;

impl RawTestServer {
    pub(super) async fn start(reply: RawReply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut acceptor = JoinSet::new();
        acceptor.spawn(accept_raw(listener, Arc::clone(&requests), reply));
        Self {
            address,
            requests,
            _acceptor: acceptor,
        }
    }

    pub(super) fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{path}", self.address)).unwrap()
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

async fn accept_raw(
    listener: TcpListener,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    reply: RawReply,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        serve_raw(socket, &requests, reply.as_ref()).await;
    }
}

async fn serve_raw(
    mut socket: TcpStream,
    requests: &Mutex<Vec<RecordedRequest>>,
    reply: &(dyn Fn(&RecordedRequest) -> RawResponse + Send + Sync),
) {
    let Some(request) = read_raw_request(&mut socket).await else {
        return;
    };
    let response = reply(&request);
    requests.lock().unwrap().push(request);
    let reason = StatusCode::from_u16(response.status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("");
    let head = format!(
        "HTTP/1.1 {} {reason}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        response.status, response.content_type, response.declared
    );
    drop(socket.write_all(head.as_bytes()).await);
    drop(socket.write_all(response.body).await);
    drop(socket.shutdown().await);
}

async fn read_raw_request(socket: &mut TcpStream) -> Option<RecordedRequest> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let head_end = read_head(socket, &mut bytes, &mut chunk).await?;
    let (method, uri, headers) = parse_head(&bytes[..head_end])?;
    let length = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0_usize);
    while bytes.len() < head_end.saturating_add(length) {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Some(RecordedRequest {
        method,
        uri,
        headers,
        body: bytes[head_end..].to_vec(),
    })
}

async fn read_head(socket: &mut TcpStream, bytes: &mut Vec<u8>, chunk: &mut [u8]) -> Option<usize> {
    loop {
        let read = socket.read(chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            return Some(position.saturating_add(4));
        }
    }
}

fn parse_head(head: &[u8]) -> Option<(Method, String, HeaderMap)> {
    let text = String::from_utf8(head.to_vec()).ok()?;
    let mut lines = text.split("\r\n");
    let mut parts = lines.next()?.split(' ');
    let method = Method::from_bytes(parts.next()?.as_bytes()).ok()?;
    let uri = parts.next()?.to_owned();
    let mut headers = HeaderMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.append(
            HeaderName::from_bytes(name.trim().as_bytes()).ok()?,
            HeaderValue::from_bytes(value.trim().as_bytes()).ok()?,
        );
    }
    Some((method, uri, headers))
}
