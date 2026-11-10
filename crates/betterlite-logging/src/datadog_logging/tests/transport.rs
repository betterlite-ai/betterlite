use std::collections::BTreeMap;
use std::str::from_utf8;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use http::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderValue};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::datadog_logging::transport::{
    LOGGING_TIMEOUT, PooledTransport, PostRequest, Transport as _, TransportFailure,
};

const ACCEPTED: &[u8] = b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a policy resolves"),
    );
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT)
        .expect("the client builds")
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut byte = [0_u8; 1];
    while !received.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .await
            .expect("the head arrives");
        received.push(byte[0]);
    }
    let head = from_utf8(&received)
        .expect("the head is text")
        .to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    stream
        .read_exact(&mut body)
        .await
        .expect("the body arrives");
    received.extend_from_slice(&body);
    String::from_utf8(received).expect("the request is text")
}

#[tokio::test]
async fn the_pooled_transport_posts_the_headers_and_body_it_was_given() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let address = listener.local_addr().expect("a local address");
    let mut tasks = JoinSet::new();
    let _server = tasks.spawn(async move {
        let (mut stream, _peer) = listener.accept().await.expect("a connection");
        let request = read_request(&mut stream).await;
        stream
            .write_all(ACCEPTED)
            .await
            .expect("the answer goes out");
        request
    });
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert("dd-api-key", HeaderValue::from_static("key-1"));
    let transport = PooledTransport { client: client() };
    let response = transport
        .post(PostRequest {
            url: format!("http://{address}/api/v2/logs"),
            headers,
            body: b"compressed-bytes".to_vec(),
            timeout: LOGGING_TIMEOUT,
        })
        .await
        .expect("the post completes");
    assert_eq!(response.status, 202);
    let request = tasks
        .join_next()
        .await
        .expect("the server task finishes")
        .expect("the server returns the request");
    assert_post(&request);
}

fn assert_post(request: &str) {
    let head = request
        .split("\r\n\r\n")
        .next()
        .expect("the request has a head")
        .to_ascii_lowercase();
    assert!(head.starts_with("post /api/v2/logs http/1.1"), "{head}");
    assert!(head.contains("content-encoding: gzip"), "{head}");
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(head.contains("dd-api-key: key-1"), "{head}");
    assert!(request.ends_with("compressed-bytes"), "{request}");
}

#[tokio::test]
async fn the_pooled_transport_reports_an_invalid_url() {
    let transport = PooledTransport { client: client() };
    let outcome = transport
        .post(PostRequest {
            url: "not a url".to_owned(),
            headers: HeaderMap::new(),
            body: Vec::new(),
            timeout: LOGGING_TIMEOUT,
        })
        .await;
    assert!(matches!(outcome, Err(TransportFailure::Url(_))));
}

#[tokio::test]
async fn the_pooled_transport_reports_a_failed_request() {
    let transport = PooledTransport { client: client() };
    let outcome = transport
        .post(PostRequest {
            url: "http://127.0.0.1:1/api/v2/logs".to_owned(),
            headers: HeaderMap::new(),
            body: Vec::new(),
            timeout: LOGGING_TIMEOUT,
        })
        .await;
    assert!(matches!(outcome, Err(TransportFailure::Request(_))));
}
