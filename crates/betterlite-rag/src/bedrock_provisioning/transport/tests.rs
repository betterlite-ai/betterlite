#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_provisioning::transport::{HttpRagTransport, HttpRagTransportError};

const CREATED: &str =
    "HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nContent-Length: 7\r\n\r\npayload";
const MISSING: &str = "HTTP/1.1 404 Not Found\r\nContent-Length: 7\r\n\r\nmissing";

fn client() -> HttpRagTransport {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the default proxy policy resolves"),
    );
    let proxied = ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT)
        .expect("the pooled client builds");
    HttpRagTransport {
        client: Arc::new(proxied),
    }
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener binds");
    let address = listener.local_addr().expect("the listener has an address");
    (listener, format!("http://{address}/"))
}

async fn serve_once(listener: TcpListener, response: &'static str) {
    let (mut stream, _peer) = listener.accept().await.expect("one connection arrives");
    let mut buffer = [0_u8; 2048];
    let _read = stream.read(&mut buffer).await.expect("the request arrives");
    stream
        .write_all(response.as_bytes())
        .await
        .expect("the answer is written");
    stream.flush().await.expect("the answer is flushed");
}

fn signed(url: &str, body: &[u8]) -> AwsSignedRequest {
    AwsSignedRequest {
        method: "POST".to_owned(),
        url: url.to_owned(),
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body: body.to_vec(),
    }
}

#[tokio::test]
async fn transport_sends_the_request_through_the_pooled_client() {
    let (listener, url) = listen().await;
    let transport = client();
    let (answer, ()) = tokio::join!(
        transport.send(signed(&url, b"{\"a\":1}")),
        serve_once(listener, CREATED)
    );
    let response = answer.expect("the request is answered");
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"payload".to_vec());
}

#[tokio::test]
async fn transport_returns_every_status_unchanged() {
    let (listener, url) = listen().await;
    let transport = client();
    let (answer, ()) = tokio::join!(
        transport.send(signed(&url, b"")),
        serve_once(listener, MISSING)
    );
    let response = answer.expect("the request is answered");
    assert_eq!(response.status, 404);
    assert_eq!(response.body, b"missing".to_vec());
}

#[tokio::test]
async fn transport_reports_a_transport_failure() {
    let (listener, url) = listen().await;
    drop(listener);
    let error = client()
        .send(signed(&url, b""))
        .await
        .expect_err("a closed port cannot answer");
    assert!(matches!(error, HttpRagTransportError::Transport { .. }));
}

#[tokio::test]
async fn transport_reports_a_bad_method() {
    let (listener, url) = listen().await;
    drop(listener);
    let request = AwsSignedRequest {
        method: "NOT A METHOD".to_owned(),
        url,
        headers: Vec::new(),
        body: Vec::new(),
    };
    let error = client()
        .send(request)
        .await
        .expect_err("the method is not HTTP");
    assert!(matches!(error, HttpRagTransportError::Method { .. }));
}

#[tokio::test]
async fn transport_reports_a_bad_url() {
    let request = AwsSignedRequest {
        method: "POST".to_owned(),
        url: "not a url".to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    let error = client()
        .send(request)
        .await
        .expect_err("the url cannot be parsed");
    assert!(matches!(error, HttpRagTransportError::Url { .. }));
}

#[tokio::test]
async fn transport_reports_a_bad_header() {
    let (listener, url) = listen().await;
    drop(listener);
    let request = AwsSignedRequest {
        method: "POST".to_owned(),
        url,
        headers: vec![("Bad Name".to_owned(), "value".to_owned())],
        body: Vec::new(),
    };
    let error = client()
        .send(request)
        .await
        .expect_err("the header name is not a token");
    assert!(matches!(error, HttpRagTransportError::Header { .. }));
}

#[test]
fn transport_debug_names_the_type_only() {
    let printed = format!("{:?}", client());
    assert!(printed.contains("HttpRagTransport"));
    assert!(!printed.contains("ProxiedClient"));
}

#[test]
fn transport_error_never_prints_a_header_value() {
    let error = HttpRagTransportError::Header {
        name: "Authorization".to_owned(),
    };
    let printed = format!("{error}");
    assert!(printed.contains("Authorization"));
    assert!(!printed.contains("AWS4-HMAC-SHA256"));
}
