#![cfg(test)]

use std::collections::BTreeMap;
use std::str::from_utf8;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use http::Method;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::transport::{HttpS3Client, S3HttpClient as _, S3Request, header_map};

const OK: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
const REJECTED: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 3\r\nconnection: close\r\n\r\nerr";

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

async fn serve(answer: &'static [u8], request: S3Request) -> (String, u16, Vec<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let address = listener.local_addr().expect("a local address");
    let mut tasks = JoinSet::new();
    let _server = tasks.spawn(async move {
        let (mut stream, _peer) = listener.accept().await.expect("a connection");
        let received = read_request(&mut stream).await;
        stream.write_all(answer).await.expect("the answer goes out");
        received
    });
    let client = HttpS3Client { client: client() };
    let url = format!("http://{address}{}", request.url);
    let sent = S3Request { url, ..request };
    let response = client.send(sent).await.expect("the send completes");
    let received = tasks
        .join_next()
        .await
        .expect("the server task finishes")
        .expect("the server returns the request");
    (received, response.status, response.body)
}

#[tokio::test]
async fn the_http_client_sends_the_request_it_was_given() {
    let request = S3Request {
        method: Method::PUT,
        url: "/logs/2026-01-02/id.json".to_owned(),
        headers: vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("x-amz-content-sha256".to_owned(), "abc".to_owned()),
        ],
        body: b"{\"id\":\"request\"}".to_vec(),
    };
    let (received, status, body) = serve(OK, request).await;
    assert_eq!(status, 200);
    assert_eq!(body, b"ok".to_vec());
    let head = received
        .split("\r\n\r\n")
        .next()
        .expect("the request has a head")
        .to_ascii_lowercase();
    assert!(
        head.starts_with("put /logs/2026-01-02/id.json http/1.1"),
        "{head}"
    );
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(head.contains("x-amz-content-sha256: abc"), "{head}");
    assert!(received.ends_with("{\"id\":\"request\"}"), "{received}");
}

#[tokio::test]
async fn the_http_client_returns_a_rejection_body() {
    let request = S3Request {
        method: Method::GET,
        url: "/logs/2026-01-02/id.json".to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    let (received, status, body) = serve(REJECTED, request).await;
    assert_eq!(status, 503);
    assert_eq!(body, b"err".to_vec());
    assert!(
        received
            .to_ascii_lowercase()
            .starts_with("get /logs/2026-01-02/id.json http/1.1"),
        "{received}"
    );
}

#[tokio::test]
async fn the_http_client_reports_an_invalid_url() {
    let client = HttpS3Client { client: client() };
    let request = S3Request {
        method: Method::GET,
        url: "not a url".to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    assert!(matches!(
        client.send(request).await,
        Err(S3TransportError::Url { .. })
    ));
}

#[test]
fn a_header_name_must_be_a_token() {
    let headers = [("bad header".to_owned(), "value".to_owned())];
    assert!(matches!(
        header_map(&headers),
        Err(S3TransportError::HeaderName { .. })
    ));
}

#[test]
fn a_header_value_must_not_carry_a_line_break() {
    let headers = [("content-disposition".to_owned(), "a\r\nb".to_owned())];
    assert!(matches!(
        header_map(&headers),
        Err(S3TransportError::HeaderValue { .. })
    ));
}

#[test]
fn header_values_are_kept_as_written() {
    let headers = [
        (
            "Content-Disposition".to_owned(),
            "inline; filename=\"a b.json\"".to_owned(),
        ),
        (
            "Content-MD5".to_owned(),
            "u+Mww//GWRAKyc7iFpGvNg==".to_owned(),
        ),
    ];
    let map = header_map(&headers).expect("the headers are valid");
    assert_eq!(
        map.get("content-disposition")
            .map(|value| value.to_str().expect("text")),
        Some("inline; filename=\"a b.json\"")
    );
    assert_eq!(
        map.get("content-md5")
            .map(|value| value.to_str().expect("text")),
        Some("u+Mww//GWRAKyc7iFpGvNg==")
    );
}
