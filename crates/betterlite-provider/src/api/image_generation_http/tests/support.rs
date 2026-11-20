#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::image_generation::transport::ImageGenerationTransport;
use http::HeaderMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use crate::api::image_generation_http::ClientSource;

const HEADER_END: &[u8] = b"\r\n\r\n";

pub(super) async fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    (listener, format!("http://{address}/v1"))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
        None
    })
}

fn request_complete(raw: &[u8]) -> bool {
    let Some(end) = find(raw, HEADER_END) else {
        return false;
    };
    let Some(head) = raw.get(..end) else {
        return false;
    };
    let length = content_length(head).unwrap_or(0);
    raw.len() >= end.saturating_add(HEADER_END.len()).saturating_add(length)
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if request_complete(&received) {
            return received;
        }
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client closed before sending the request");
        received.extend_from_slice(chunk.get(..read).expect("the bytes read"));
    }
}

// One HTTP/1.1 answer over a byte channel: the status reason, the headers, then a counted body
pub(super) fn reply(
    status: &'static str,
    headers: &[(&'static str, &'static [u8])],
    body: &'static [u8],
) -> Vec<u8> {
    let mut raw = format!("HTTP/1.1 {status}\r\n").into_bytes();
    for (name, value) in headers {
        raw.extend_from_slice(name.as_bytes());
        raw.extend_from_slice(b": ");
        raw.extend_from_slice(value);
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(format!("content-length: {}\r\n", body.len()).as_bytes());
    raw.extend_from_slice(b"connection: close\r\n\r\n");
    raw.extend_from_slice(body);
    raw
}

pub(super) struct Captured {
    raw: Vec<u8>,
}

impl Captured {
    fn head(&self) -> &str {
        let end = find(&self.raw, HEADER_END).expect("the header terminator");
        std::str::from_utf8(self.raw.get(..end).expect("the header bytes"))
            .expect("the header text")
    }

    pub(super) fn body(&self) -> &[u8] {
        let end = find(&self.raw, HEADER_END).expect("the header terminator");
        self.raw
            .get(end.saturating_add(HEADER_END.len())..)
            .expect("the body bytes")
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(self.body().to_vec()).expect("the body text")
    }

    pub(super) fn request_line(&self) -> &str {
        self.head().lines().next().expect("the request line")
    }

    pub(super) fn header(&self, wanted: &str) -> Option<String> {
        self.head().lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(wanted)
                .then(|| value.trim().to_owned())
        })
    }

    fn boundary(&self) -> Option<String> {
        let content_type = self.header("content-type")?;
        let (_, boundary) = content_type.split_once("boundary=")?;
        Some(boundary.to_owned())
    }

    // the body with its random multipart boundary spelled the same way on every run
    pub(super) fn normalized_body(&self) -> String {
        let text = self.text();
        match self.boundary() {
            Some(boundary) => text.replace(&boundary, "<boundary>"),
            None => text,
        }
    }
}

pub(super) async fn serve_once(listener: &TcpListener, reply: &[u8]) -> Captured {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let raw = read_request(&mut stream).await;
    stream.write_all(reply).await.expect("the reply");
    Captured { raw }
}

// Holds the connection open past the client's deadline without ever answering
pub(super) async fn stall_once(listener: &TcpListener, hold: Duration) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let _read = stream.read(&mut chunk).await.expect("the request bytes");
    tokio::time::sleep(hold).await;
}

#[derive(Clone)]
pub(super) struct StubClient {
    status: u16,
    body: &'static [u8],
    sent: Arc<Mutex<Vec<OutboundRequest>>>,
}

impl StubClient {
    pub(super) fn new(status: u16, body: &'static [u8]) -> Self {
        Self {
            status,
            body,
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn sent(&self) -> usize {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl ImageGenerationTransport for StubClient {
    fn generate(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        std::future::ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.to_vec(),
        }))
    }
}

// A pooled client built the way production builds it, recording the ssl_verify each request asked for
pub(super) struct PoolSource {
    client: ProxiedClient,
    seen: Mutex<Vec<Option<bool>>>,
}

impl PoolSource {
    pub(super) fn new() -> Self {
        let policy = Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
                .expect("the proxy policy"),
        );
        Self {
            client: ProxiedClient::outbound(&policy).expect("the outbound client"),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<Option<bool>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for PoolSource {
    type Client = ProxiedClient;
    type Error = Infallible;

    fn client(&self, ssl_verify: Option<bool>) -> Result<ProxiedClient, Infallible> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.client.clone())
    }
}

// A client the shared path builds, so a case can tell which of the two answered
pub(super) struct StubSource {
    client: StubClient,
    seen: Mutex<Vec<Option<bool>>>,
}

impl StubSource {
    pub(super) fn new(client: StubClient) -> Self {
        Self {
            client,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn stub(&self) -> &StubClient {
        &self.client
    }

    pub(super) fn seen(&self) -> Vec<Option<bool>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for StubSource {
    type Client = StubClient;
    type Error = Infallible;

    fn client(&self, ssl_verify: Option<bool>) -> Result<StubClient, Infallible> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.client.clone())
    }
}

// A client whose send fails with no status, the way a shutdown or a refused connection arrives
pub(super) struct ClosedClient;

impl ImageGenerationTransport for ClosedClient {
    fn generate(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        std::future::ready(Err(TransportError::Closed))
    }
}

pub(super) struct ClosedSource;

impl ClientSource for ClosedSource {
    type Client = ClosedClient;
    type Error = Infallible;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ClosedClient, Infallible> {
        Ok(ClosedClient)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the test source refuses to build a client")]
pub(super) struct SourceRefusal;

// A source that cannot hand out a client at all
pub(super) struct FailingSource;

impl ClientSource for FailingSource {
    type Client = StubClient;
    type Error = SourceRefusal;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<StubClient, SourceRefusal> {
        Err(SourceRefusal)
    }
}
