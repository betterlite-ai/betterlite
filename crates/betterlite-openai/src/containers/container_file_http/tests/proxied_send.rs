#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use crate::containers::container_file_http::proxied::ProxiedContainerClient;
use crate::containers::container_file_http::transport::{
    AsyncClient as _, ContainerHttpRequest, ContainerMethod, UploadedFile,
};

fn client() -> ProxiedContainerClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedContainerClient {
        client: ProxiedClient::outbound(&policy).unwrap(),
        deadline: Duration::from_secs(30),
    }
}

fn request(method: ContainerMethod, url: String) -> ContainerHttpRequest {
    ContainerHttpRequest {
        method,
        url,
        headers: BTreeMap::from([("Authorization".to_owned(), "Bearer k".to_owned())]),
        query: Vec::new(),
        file: None,
    }
}

// Answers one request and returns the request text it read
async fn serve_once(listener: TcpListener, status: u16, body: &str) -> String {
    let (mut stream, _peer) = listener.accept().await.unwrap();
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
        "HTTP/1.1 {status} X\r\nx-request-id: req_9\r\nx-a: one\r\nx-a: two\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    format!("{head}{}", String::from_utf8(sent).unwrap())
}

#[tokio::test]
async fn send_makes_the_request_and_returns_status_headers_and_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/v1/containers/c1/files?api-version=v1",
        listener.local_addr().unwrap()
    );
    let server = serve_once(listener, 404, r#"{"error":{"message":"gone"}}"#);
    let mut asked = request(ContainerMethod::Delete, url);
    asked.query = vec![("limit".to_owned(), "5".to_owned())];
    let sender = client();
    let (outcome, seen) = tokio::join!(sender.send(&asked), server);
    let reply = outcome.unwrap();
    assert_eq!(reply.status, 404);
    assert_eq!(reply.body, br#"{"error":{"message":"gone"}}"#);
    assert_eq!(
        reply.headers.get("x-request-id").map(String::as_str),
        Some("req_9")
    );
    assert_eq!(
        reply.headers.get("x-a").map(String::as_str),
        Some("one, two")
    );
    assert!(
        seen.starts_with("DELETE /v1/containers/c1/files?api-version=v1&limit=5 HTTP/1.1"),
        "{seen}"
    );
    assert!(seen.contains("authorization: Bearer k"), "{seen}");
}

#[tokio::test]
async fn send_posts_the_file_as_multipart_form_data() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/v1/containers/c1/files",
        listener.local_addr().unwrap()
    );
    let server = serve_once(listener, 200, "{}");
    let mut upload = request(ContainerMethod::Post, url);
    upload.file = Some(UploadedFile {
        filename: "a.txt".to_owned(),
        content: b"hello".to_vec(),
        content_type: "text/plain".to_owned(),
    });
    let sender = client();
    let (reply, seen) = tokio::join!(sender.send(&upload), server);
    assert_eq!(reply.unwrap().status, 200);
    assert!(
        seen.starts_with("POST /v1/containers/c1/files HTTP/1.1"),
        "{seen}"
    );
    assert!(
        seen.contains("content-type: multipart/form-data; boundary=betterlite-"),
        "{seen}"
    );
    assert!(seen.contains(r#"name="file"; filename="a.txt""#), "{seen}");
    assert!(seen.contains("hello"), "{seen}");
}
