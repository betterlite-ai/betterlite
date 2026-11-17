use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::RequestBody;
use futures_util::StreamExt as _;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::data_cloud_dispatch::transport::{Pooled, PostTransport as _, WireRequest};

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the fixture proxy policy builds");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the fixture client builds")
}

// answers one request on the listener with the given status line and body, and returns the request text
async fn serve_once(listener: &TcpListener, status: &str, body: &str) -> String {
    let (mut stream, _peer) = listener
        .accept()
        .await
        .expect("the fixture accepts the request");
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.expect("the request arrives");
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8(request.clone()).expect("the request is UTF-8");
        if let Some((head, sent)) = text.split_once("\r\n\r\n") {
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    let length = value.trim().parse::<usize>().ok()?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(length)
                })
                .unwrap_or(0);
            if sent.len() >= length {
                break;
            }
        }
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\nx-dispatch: 1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(reply.as_bytes())
        .await
        .expect("the reply is written");
    String::from_utf8(request).expect("the request is UTF-8")
}

fn post_request<'request>(
    url: &'request str,
    headers: &'request HeaderMap,
) -> WireRequest<'request> {
    WireRequest {
        method: Method::POST,
        url,
        headers,
        body: b"{\"model\":\"m\"}",
        deadline: EXCHANGE_TIMEOUT,
    }
}

#[test]
fn the_wire_request_carries_its_parts_to_the_outbound_request() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-call"),
        HeaderValue::from_static("1"),
    );
    let request = post_request("http://mock/chat/completions", &headers);
    let outbound = request.outbound().expect("the request converts");
    assert_eq!(outbound.method, Method::POST);
    assert_eq!(outbound.url.as_str(), "http://mock/chat/completions");
    assert!(outbound.query.is_empty());
    assert_eq!(
        outbound
            .headers
            .get("x-call")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
    assert_eq!(outbound.deadline, EXCHANGE_TIMEOUT);
    let RequestBody::Json(body) = &outbound.body else {
        panic!("the wire body is JSON");
    };
    assert_eq!(body.as_slice(), b"{\"model\":\"m\"}");
}

#[tokio::test]
async fn the_pooled_transport_returns_the_status_headers_and_body() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the fixture binds");
    let url = format!(
        "http://{}/chat/completions",
        listener.local_addr().expect("the fixture address")
    );
    let client = client();
    let pooled = Pooled { client: &client };
    let headers = HeaderMap::new();
    let request = post_request(&url, &headers);
    let exchange = async {
        tokio::join!(
            serve_once(&listener, "404 Not Found", r#"{"error":"missing"}"#),
            pooled.post(&request)
        )
    };
    let (sent, answer) = timeout(EXCHANGE_TIMEOUT, exchange)
        .await
        .expect("the exchange finishes");
    let response = answer.expect("the transport answers");
    assert_eq!(response.status, 404);
    assert_eq!(
        response
            .headers
            .get("x-dispatch")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
    assert_eq!(response.body.as_slice(), br#"{"error":"missing"}"#);
    assert!(sent.starts_with("POST /chat/completions HTTP/1.1\r\n"));
    assert!(sent.contains("\"model\":\"m\""));
}

#[tokio::test]
async fn the_pooled_stream_returns_the_body_of_a_failed_status() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the fixture binds");
    let url = format!(
        "http://{}/chat/completions",
        listener.local_addr().expect("the fixture address")
    );
    let client = client();
    let pooled = Pooled { client: &client };
    let headers = HeaderMap::new();
    let request = post_request(&url, &headers);
    let exchange = async {
        tokio::join!(
            serve_once(&listener, "500 Internal Server Error", "boom"),
            pooled.post_stream(&request)
        )
    };
    let (sent, answer) = timeout(EXCHANGE_TIMEOUT, exchange)
        .await
        .expect("the exchange finishes");
    let head = answer.expect("the transport answers");
    assert_eq!(head.status, 500);
    assert_eq!(head.body, b"boom".to_vec());
    assert!(head.chunks.is_none());
    assert!(sent.starts_with("POST /chat/completions HTTP/1.1\r\n"));
}

#[tokio::test]
async fn the_pooled_stream_returns_the_chunks_of_a_success() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the fixture binds");
    let url = format!(
        "http://{}/chat/completions",
        listener.local_addr().expect("the fixture address")
    );
    let client = client();
    let pooled = Pooled { client: &client };
    let headers = HeaderMap::new();
    let request = post_request(&url, &headers);
    let exchange = async {
        tokio::join!(
            serve_once(&listener, "200 OK", "data: [DONE]\n\n"),
            pooled.post_stream(&request)
        )
    };
    let (_sent, answer) = timeout(EXCHANGE_TIMEOUT, exchange)
        .await
        .expect("the exchange finishes");
    let head = answer.expect("the transport answers");
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    let chunks = head.chunks.expect("a successful stream holds chunks");
    let collected: Vec<Vec<u8>> = chunks
        .map(|chunk| chunk.expect("the chunk arrives"))
        .collect()
        .await;
    assert_eq!(collected.concat(), b"data: [DONE]\n\n".to_vec());
}
