#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use crate::bedrock_post::{PostError, sign_and_post};
use crate::bedrock_transport::test_support::{RecordingLog, guardrail_with, prepared_request};

// serves one request and returns what the client sent
async fn serve_once(listener: TcpListener, reply: &'static str) -> String {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let body = prepared_request().body;
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !received.ends_with(&body) {
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client closed before sending the body");
        received.extend_from_slice(chunk.get(..read).expect("the bytes read"));
    }
    stream.write_all(reply.as_bytes()).await.expect("the reply");
    String::from_utf8(received).expect("the request text")
}

async fn post_to_local_server(reply: &'static str) -> (Result<u16, PostError>, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    let client = ProxiedClient::outbound(&policy).expect("the client");
    let mut request = prepared_request();
    request.url = format!("http://{address}/guardrail/gr-1/version/1/apply");
    let guardrail = guardrail_with(OpaqueParams::default());
    let log = RecordingLog::default();
    let (result, sent) = tokio::join!(
        sign_and_post(&guardrail, &request, &client, &log, true),
        serve_once(listener, reply)
    );
    (result.map(|response| response.status), sent)
}

#[tokio::test]
async fn proxied_transport_posts_the_signed_request_over_http() {
    let reply = "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";
    let (result, sent) = post_to_local_server(reply).await;
    assert_eq!(result.expect("the response"), 200);
    let lowercase = sent.to_ascii_lowercase();
    assert!(sent.starts_with("POST /guardrail/gr-1/version/1/apply HTTP/1.1"));
    assert!(lowercase.contains("content-type: application/json"));
    assert!(lowercase.contains("authorization: aws4-hmac-sha256"));
    assert!(sent.ends_with(r#"{"source":"INPUT","content":[]}"#));
}

#[tokio::test]
async fn proxied_transport_reports_the_http_error_body() {
    let reply = "HTTP/1.1 400 Bad Request\r\ncontent-length: 23\r\nconnection: close\r\n\r\n{\"message\":\"bad input\"}";
    let (result, _sent) = post_to_local_server(reply).await;
    let error = result.expect_err("the HTTP error");
    let PostError::Http { status, detail } = error else {
        panic!("expected the HTTP error, got {error}");
    };
    assert_eq!(status, 400);
    assert_eq!(detail, "bad input");
}
