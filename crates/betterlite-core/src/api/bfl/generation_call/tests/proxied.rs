use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::sleep;
use url::Url;

use crate::api::bfl::generation_call::submit::{SubmitRequest, SubmitTransport as _};
use crate::api::bfl::poll::PollTransport as _;

const REPLY: &str = "HTTP/1.1 201 Created\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";
const SHORT_READ_LIMIT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(500),
    ..TransportLimits::DEFAULT
};
const HOLD: Duration = Duration::from_secs(2);

// serves one request, waits until the client has sent the tail, and returns what it sent
async fn serve_once(listener: TcpListener, tail: &[u8]) -> String {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !received.ends_with(tail) {
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client closed before sending the request");
        received.extend_from_slice(chunk.get(..read).expect("the bytes read"));
    }
    stream.write_all(REPLY.as_bytes()).await.expect("the reply");
    String::from_utf8(received).expect("the request text")
}

async fn local_server(limits: TransportLimits) -> (TcpListener, SocketAddr, ProxiedClient) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    let client =
        ProxiedClient::build(base_builder().http1_only(), &policy, limits).expect("the client");
    (listener, address, client)
}

fn key_header() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-key", HeaderValue::from_static("sk-bfl"));
    headers
}

#[tokio::test]
async fn the_provider_client_posts_the_json_body_with_the_headers() {
    let (listener, address, client) = local_server(TransportLimits::DEFAULT).await;
    let body = Json::Object(IndexMap::from([(
        String::from("prompt"),
        Json::Text(String::from("a cat")),
    )]));
    let request = SubmitRequest {
        url: Url::parse(&format!("http://{address}/v1/flux-pro-1.1")).expect("the URL"),
        headers: key_header(),
        body,
        timeout: None,
    };

    let (answered, sent) = tokio::join!(
        client.post(request),
        serve_once(listener, br#"{"prompt":"a cat"}"#)
    );

    let response = answered.expect("the submission is answered");
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"{}");
    assert!(sent.starts_with("POST /v1/flux-pro-1.1 HTTP/1.1"));
    assert!(sent.to_ascii_lowercase().contains("x-key: sk-bfl"));
    assert!(
        sent.to_ascii_lowercase()
            .contains("content-type: application/json")
    );
}

#[tokio::test]
async fn the_provider_client_polls_with_a_get_carrying_the_headers() {
    let (listener, address, client) = local_server(TransportLimits::DEFAULT).await;
    let url = Url::parse(&format!("http://{address}/v1/get_result?id=1")).expect("the URL");

    let (answered, sent) = tokio::join!(
        client.get(url, key_header()),
        serve_once(listener, b"\r\n\r\n")
    );

    let response = answered.expect("the poll is answered");
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"{}");
    assert!(sent.starts_with("GET /v1/get_result?id=1 HTTP/1.1"));
    assert!(sent.to_ascii_lowercase().contains("x-key: sk-bfl"));
}

// keeps the connection open without answering, so only the client's own deadline can end the request
async fn hold_open(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.expect("the request bytes");
    assert_ne!(
        read, 0,
        "the client sends its request before the read limit"
    );
    sleep(HOLD).await;
}

// a poll is bounded by the client's own read limit, never by the submission's caller timeout
#[tokio::test]
async fn the_provider_client_bounds_a_poll_with_its_own_read_limit() {
    let (listener, address, client) = local_server(SHORT_READ_LIMIT).await;
    let url = Url::parse(&format!("http://{address}/v1/get_result?id=1")).expect("the URL");

    let (result, ()) = tokio::join!(client.get(url, key_header()), hold_open(listener));

    let error = result.expect_err("the read limit ends the poll");
    assert!(matches!(error, TransportError::Deadline(_)));
}
