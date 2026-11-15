#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use futures_util::StreamExt as _;
use http::Method;
use http::header::HeaderMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use url::Url;

use crate::gemini_http::call::{Clients, async_completion, async_streaming};
use crate::gemini_http::error::GeminiStreamError;
use crate::gemini_http::post::GeminiPost;
use crate::gemini_http::tests::support::{
    ANSWER, BODY, Piece, Recorder, STREAM_CHUNK, TOKEN, auth, catalog, llm, object, pooled,
    stream_reply, vertex_call,
};

use super::fixed_id;

fn pooled_proxy() -> Clients {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let transport: Arc<dyn GeminiPost> = Arc::new(ProxiedClient::outbound(&policy).unwrap());
    Clients {
        supplied: None,
        pooled: transport,
    }
}

#[tokio::test]
async fn execution_posts_through_the_pooled_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let api_base = format!("http://{address}/");
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.api_base = Some(&api_base);
    let clients = pooled_proxy();
    let fixture = auth();
    let mut server = JoinSet::new();
    server.spawn(answer_complete(listener));
    let response = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    assert_eq!(response.usage.total_tokens, 5);
    let received = timeout(Duration::from_secs(5), server.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let request = String::from_utf8(received).unwrap();
    assert!(request.starts_with(
        "POST /v1/projects/project-p/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent HTTP/1.1"
    ));
    assert!(request.contains(&format!("authorization: Bearer {TOKEN}")));
    assert!(request.contains("content-type: application/json"));
    assert!(request.ends_with(BODY));
}

#[tokio::test]
async fn execution_streams_through_the_pooled_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let api_base = format!("http://{address}/");
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.api_base = Some(&api_base);
    let clients = pooled_proxy();
    let fixture = auth();
    let mut server = JoinSet::new();
    server.spawn(answer_stream(listener));
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let chunk = timeout(Duration::from_secs(5), stream.next_chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(chunk.id, "r1");
    assert!(stream.next_chunk().await.is_none());
    let received = timeout(Duration::from_secs(5), server.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let request = String::from_utf8(received).unwrap();
    assert!(request.starts_with(
        "POST /v1/projects/project-p/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse HTTP/1.1"
    ));
    assert!(request.contains(&format!("authorization: Bearer {TOKEN}")));
    assert!(request.ends_with(BODY));
}

#[tokio::test]
async fn execution_streaming_body_timeout_is_typed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let outbound = ProxiedClient::outbound(&policy).unwrap();
    let mut server = JoinSet::new();
    server.spawn(stall_body(listener));
    let response = outbound
        .send(OutboundRequest {
            method: Method::POST,
            url: Url::parse(&format!("http://{address}/generateContent")).unwrap(),
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: Duration::from_secs(1),
        })
        .await
        .unwrap();
    let failure = response.body_stream().next().await.unwrap().unwrap_err();
    assert!(matches!(&failure, HttpError::Request(source) if source.is_timeout()));
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![Piece::Failed(failure)])],
    ));
    let clients = pooled(&recorder);
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    assert!(matches!(error, GeminiStreamError::Timeout));
    drop(server);
}

async fn answer_complete(listener: TcpListener) -> Vec<u8> {
    let (mut stream, _) = listener.accept().await.unwrap();
    let received = read_request(&mut stream).await;
    write_body(&mut stream, "application/json", ANSWER).await;
    received
}

async fn answer_stream(listener: TcpListener) -> Vec<u8> {
    let (mut stream, _) = listener.accept().await.unwrap();
    let received = read_request(&mut stream).await;
    write_body(
        &mut stream,
        "text/event-stream",
        &format!("{STREAM_CHUNK}\n\n"),
    )
    .await;
    received
}

// the head arrives at once, the ten bytes it announces never do
async fn stall_body(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let _ = read_request(&mut stream).await;
    stream
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\nconnection: close\r\n\r\n")
        .await
        .unwrap();
    sleep(Duration::from_secs(5)).await;
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 4096];
    while !request_is_complete(&received) {
        let read = stream.read(&mut buffer).await.unwrap();
        assert_ne!(
            read, 0,
            "the client closed before sending its whole request"
        );
        received.extend_from_slice(&buffer[..read]);
    }
    received
}

async fn write_body(stream: &mut TcpStream, content_type: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
}

fn request_is_complete(received: &[u8]) -> bool {
    let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = str::from_utf8(&received[..head_end]).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(": ")?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or_default();
    received.len() >= head_end.saturating_add(4).saturating_add(length)
}
