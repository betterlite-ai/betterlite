#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;

use crate::anthropic_http_stream::call::{OpenedStream, StreamCall, StreamClients, StreamOpening};
use crate::anthropic_http_stream::error::{StreamFailure, StreamOpenError};
use crate::stream_decode::DecodedChunk;
use crate::stream_decode::error::DecodeError;

use super::fixture::{
    DATA, ERROR_TEXT, FRAME, FixedCounter, FixedIds, SilentLogger, header, messages,
    request_headers, stream_build,
};

const TRUNCATED_HEAD: &str =
    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 4096\r\n\r\n";
const TRICKLE_HEAD: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
const TRICKLE_FRAMES: usize = 20;
const TRICKLE_STEP: Duration = Duration::from_millis(100);
const SHORT_TIMEOUT: Duration = Duration::from_secs(1);
const LONG_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn anthropic_http_stream_partial_body_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_truncated(listener));
    let client = proxied_client();
    let drain_client = client.clone();
    let mut opened = open_with(&api_base, &client, Duration::from_secs(10))
        .await
        .expect("the local server answers 200");
    let first = timeout(Duration::from_secs(10), opened.completion_stream.next())
        .await
        .expect("the first frame arrives before the close")
        .expect("the frame is read")
        .expect("the frame decodes");
    assert!(matches!(first, DecodedChunk::Event(_)));
    let ended = timeout(Duration::from_secs(10), opened.completion_stream.next())
        .await
        .expect("a truncated body ends the stream instead of hanging")
        .expect("the truncation is reported");
    assert!(matches!(ended, Err(DecodeError::Other { .. })));
    drop(opened);
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains after the truncation");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn anthropic_http_stream_cancel_upstream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let (closed, closed_rx) = oneshot::channel::<()>();
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_until_closed(listener, closed));
    let client = proxied_client();
    let drain_client = client.clone();
    let mut opened = open_with(&api_base, &client, Duration::from_secs(10))
        .await
        .expect("the local server answers 200");
    let first = timeout(Duration::from_secs(10), opened.completion_stream.next())
        .await
        .expect("the first frame arrives")
        .expect("the frame is read")
        .expect("the frame decodes");
    assert!(matches!(first, DecodedChunk::Event(_)));
    drop(opened);
    timeout(Duration::from_secs(10), closed_rx)
        .await
        .expect("the dropped stream releases the connection")
        .unwrap();
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains after the drop");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the upstream server ends after the drop")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn anthropic_http_stream_proxied_status_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_response(listener, status_response(400, ERROR_TEXT)));
    let client = proxied_client();
    let drain_client = client.clone();
    let Err(error) = open_with(&api_base, &client, Duration::from_secs(10)).await else {
        panic!("a 400 reply opens no stream")
    };
    let StreamOpenError::Anthropic {
        status_code,
        message,
        headers,
    } = error
    else {
        panic!("a 400 reply becomes AnthropicError");
    };
    assert_eq!(status_code, 400);
    assert_eq!(message, ERROR_TEXT);
    let carried = headers.expect("the failure keeps the reply headers");
    assert_eq!(header(&carried, "x-request-id"), Some("req-proxied"));
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn anthropic_http_stream_proxied_open_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_silence(listener));
    let client = proxied_client();
    let drain_client = client.clone();
    let Err(error) = open_with(&api_base, &client, SHORT_TIMEOUT).await else {
        panic!("a server that never answers opens no stream")
    };
    let StreamOpenError::Reraised { source } = error else {
        panic!("a send timeout is re-raised");
    };
    let StreamFailure::Transport(failure) = source else {
        panic!("the request failed in the transport");
    };
    assert!(matches!(failure, TransportError::Deadline(_)));
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn anthropic_http_stream_proxied_connect_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    drop(listener);
    let client = proxied_client();
    let Err(error) = open_with(&api_base, &client, SHORT_TIMEOUT).await else {
        panic!("a refused connection opens no stream")
    };
    let StreamOpenError::Unknown { source } = error else {
        panic!("a refused connection becomes AnthropicError(500)");
    };
    let StreamFailure::Transport(failure) = source else {
        panic!("the request failed in the transport");
    };
    assert!(matches!(failure, TransportError::Connect(_)));
    client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains");
}

#[tokio::test(start_paused = true)]
async fn anthropic_http_stream_proxied_error_body_stalls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_stalled_body(listener, 500));
    let client = proxied_client();
    let drain_client = client.clone();
    let Err(error) = open_with(&api_base, &client, SHORT_TIMEOUT).await else {
        panic!("a 500 reply opens no stream")
    };
    assert_status(error, 500);
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn anthropic_http_stream_proxied_error_body_truncated() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_short_body(listener, 500));
    let client = proxied_client();
    let drain_client = client.clone();
    let Err(error) = open_with(&api_base, &client, SHORT_TIMEOUT).await else {
        panic!("a 500 reply opens no stream")
    };
    assert_status(error, 500);
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

// a rate limit whose body cannot be read stays a rate limit, as LiteLLM's masked handler keeps the status
#[tokio::test]
async fn anthropic_http_stream_proxied_error_body_truncated_keeps_429() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_short_body(listener, 429));
    let client = proxied_client();
    let drain_client = client.clone();
    let Err(error) = open_with(&api_base, &client, SHORT_TIMEOUT).await else {
        panic!("a 429 reply opens no stream")
    };
    assert_status(error, 429);
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

// LiteLLM caps the streaming error body read at five seconds, so a stalled body ends well before a long call deadline
#[tokio::test(start_paused = true)]
async fn anthropic_http_stream_error_body_read_capped() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_stalled_body(listener, 500));
    let client = proxied_client();
    let drain_client = client.clone();
    let Err(error) = timeout(
        Duration::from_secs(9),
        open_with(&api_base, &client, LONG_TIMEOUT),
    )
    .await
    .expect("the error body read is capped instead of waiting for the call deadline") else {
        panic!("a 500 reply opens no stream")
    };
    assert_status(error, 500);
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

fn assert_status(error: StreamOpenError<StreamFailure>, expected: u16) {
    let StreamOpenError::Anthropic {
        status_code,
        message,
        headers,
    } = error
    else {
        panic!("a status error keeps the upstream status");
    };
    assert_eq!(status_code, expected);
    assert!(message.is_empty());
    let carried = headers.expect("the failure keeps the reply headers");
    assert_eq!(header(&carried, "x-request-id"), Some("req-proxied"));
}

#[tokio::test]
async fn anthropic_http_stream_total_deadline_ends_a_trickle() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_trickle(listener, TRICKLE_FRAMES));
    let client = proxied_client();
    let drain_client = client.clone();
    let mut opened = open_with(&api_base, &client, SHORT_TIMEOUT)
        .await
        .expect("the local server answers 200");
    let (received, ended) = read_until_error(&mut opened).await;
    assert!(
        received < TRICKLE_FRAMES,
        "the stream stopped while the server still had frames to send"
    );
    let DecodeError::Other { source } = ended else {
        panic!("the deadline ends the stream as a transport error");
    };
    let StreamFailure::Lines(AlephAlphaError::Transport { source: reason }) = source else {
        panic!("the body read fails in the transport");
    };
    assert!(matches!(reason, TransportError::Rejected(_)));
    drop(opened);
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains after the deadline");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the upstream server ends after the deadline")
        .unwrap()
        .unwrap();
}

async fn read_until_error(
    opened: &mut OpenedStream<StreamFailure, FixedCounter>,
) -> (usize, DecodeError<StreamFailure>) {
    let mut received = 0_usize;
    loop {
        let item = timeout(Duration::from_secs(10), opened.completion_stream.next())
            .await
            .expect("the deadline ends the stream instead of waiting for the last frame")
            .expect("the stream reports the deadline instead of ending cleanly");
        match item {
            Ok(_chunk) => received = received.saturating_add(1),
            Err(error) => return (received, error),
        }
    }
}

async fn open_with(
    api_base: &str,
    client: &ProxiedClient,
    timeout: Duration,
) -> Result<OpenedStream<StreamFailure, FixedCounter>, StreamOpenError<StreamFailure>> {
    let messages = messages();
    let reverse = IndexMap::new();
    let call = StreamCall {
        api_base,
        headers: request_headers(),
        data: DATA,
        messages: &messages,
        timeout,
        json_mode: false,
        speed: None,
        tool_name_reverse_map: &reverse,
    };
    let logger = SilentLogger;
    StreamOpening {
        clients: StreamClients {
            supplied: Some(client),
            module_level_aclient: client,
            module_level_client: client,
            logger: &logger,
        },
        call: &call,
        build: stream_build(&FixedIds),
    }
    .make_call()
    .await
}

fn status_response(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Error\r\ncontent-type: application/json\r\nx-request-id: req-proxied\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
}

async fn accept_with_head(listener: TcpListener, head: &str) -> TcpStream {
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(head.as_bytes()).await.unwrap();
    socket
}

async fn drain_socket(socket: &mut TcpStream) {
    let mut rest = Vec::new();
    let _closed = socket.read_to_end(&mut rest).await;
}

async fn serve_body(listener: TcpListener, head: &str, body: &str, close: bool) {
    let mut socket = accept_with_head(listener, head).await;
    socket.write_all(body.as_bytes()).await.unwrap();
    if close {
        socket.shutdown().await.unwrap();
    }
    drain_socket(&mut socket).await;
}

async fn serve_response(listener: TcpListener, response: String) {
    serve_body(listener, &response, "", true).await;
}

async fn serve_silence(listener: TcpListener) {
    let mut socket = accept_with_head(listener, "").await;
    drain_socket(&mut socket).await;
}

async fn serve_stalled_body(listener: TcpListener, status: u16) {
    let head = stall_head(status);
    serve_body(listener, &head, "partial", false).await;
}

async fn serve_short_body(listener: TcpListener, status: u16) {
    let head = stall_head(status);
    serve_body(listener, &head, "partial", true).await;
}

fn stall_head(status: u16) -> String {
    format!(
        "HTTP/1.1 {status} Error\r\ncontent-type: application/json\r\nx-request-id: req-proxied\r\ncontent-length: 4096\r\n\r\n"
    )
}

async fn serve_trickle(listener: TcpListener, frames: usize) {
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(TRICKLE_HEAD.as_bytes()).await.unwrap();
    for _frame in 0..frames {
        if socket
            .write_all(format!("{FRAME}\n").as_bytes())
            .await
            .is_err()
        {
            return;
        }
        sleep(TRICKLE_STEP).await;
    }
}

async fn serve_truncated(listener: TcpListener) {
    let line = format!("{FRAME}\n");
    serve_body(listener, TRUNCATED_HEAD, &line, true).await;
}

async fn serve_until_closed(listener: TcpListener, closed: oneshot::Sender<()>) {
    let line = format!("{FRAME}\n");
    serve_body(listener, TRUNCATED_HEAD, &line, false).await;
    let _sent = closed.send(());
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}
