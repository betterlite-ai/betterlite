#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::{HttpTransport, Post, Transport as _, TransportFailure};
use crate::copilot::device_auth::github_headers;
use crate::openai_base_config::Headers;

const DEVICE_URL: &str = "https://device.example/code";

fn policy() -> Arc<ProxyPolicy> {
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap())
}

fn http_transport() -> HttpTransport {
    HttpTransport {
        client: ProxiedClient::outbound(&policy()).unwrap(),
        deadline: Duration::from_secs(5),
    }
}

async fn serve(listener: TcpListener, status: u16, body: &'static str) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    stream.read_exact(&mut sent).await.unwrap();
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _written = stream.write_all(reply.as_bytes()).await;
    format!("{head}{}", String::from_utf8(sent).unwrap())
}

#[tokio::test]
async fn http_transport_posts_json_with_its_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut transport = http_transport();
    let url = format!("http://{address}/login/device/code");
    let body = br#"{"client_id":"c","scope":"read:user"}"#.to_vec();
    let post = Post {
        url: &url,
        headers: github_headers(None),
        body,
    };

    let (received, answered) = tokio::join!(
        serve(listener, 200, r#"{"device_code":"d"}"#),
        transport.post_json(post)
    );

    let reply = answered.unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, br#"{"device_code":"d"}"#);
    assert!(received.starts_with("POST /login/device/code HTTP/1.1\r\n"));
    assert!(received.contains("accept: application/json"));
    assert!(received.contains("editor-version: vscode/1.85.1"));
    assert!(received.contains("content-type: application/json"));
    assert!(received.ends_with(r#"{"client_id":"c","scope":"read:user"}"#));
}

#[tokio::test]
async fn http_transport_returns_error_statuses_as_replies() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut transport = http_transport();
    let url = format!("http://{address}/login/oauth/access_token");
    let post = Post {
        url: &url,
        headers: Headers::new(),
        body: b"{}".to_vec(),
    };

    let (_received, answered) = tokio::join!(
        serve(listener, 400, r#"{"error":"bad_verification_code"}"#),
        transport.post_json(post)
    );

    let reply = answered.unwrap();
    assert_eq!(reply.status, 400);
}

#[tokio::test]
async fn http_transport_rejects_an_invalid_url() {
    let mut transport = http_transport();
    let post = Post {
        url: "not a url",
        headers: Headers::new(),
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(error, TransportFailure::Url(_)));
}

#[tokio::test]
async fn http_transport_rejects_an_invalid_header_name() {
    let mut transport = http_transport();
    let mut headers = Headers::new();
    headers.insert(String::from("bad header"), String::from("value"));
    let post = Post {
        url: "http://127.0.0.1:1/login/device/code",
        headers,
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(
        error,
        TransportFailure::HeaderName { name, .. } if name == "bad header"
    ));
}

#[tokio::test]
async fn http_transport_rejects_an_invalid_header_value() {
    let mut transport = http_transport();
    let mut headers = Headers::new();
    headers.insert(String::from("accept"), String::from("bad\nvalue"));
    let post = Post {
        url: "http://127.0.0.1:1/login/device/code",
        headers,
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(
        error,
        TransportFailure::HeaderValue { name, .. } if name == "accept"
    ));
}

#[tokio::test]
async fn http_transport_reports_a_refused_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut transport = http_transport();
    let url = format!("http://{address}/login/device/code");
    let post = Post {
        url: &url,
        headers: Headers::new(),
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(error, TransportFailure::Request(_)));
}

#[test]
fn post_debug_hides_header_values() {
    let mut headers = Headers::new();
    headers.insert(
        String::from("authorization"),
        String::from("token private-access-token"),
    );
    let post = Post {
        url: DEVICE_URL,
        headers,
        body: b"{}".to_vec(),
    };

    let debug = format!("{post:?}");

    assert!(!debug.contains("private-access-token"));
    assert!(debug.contains("authorization"));
    assert!(debug.contains("body_bytes"));
}
