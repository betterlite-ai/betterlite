use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use futures_util::{Stream, StreamExt as _};
use secrecy::SecretString;
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use super::{Event, Recorder};
use crate::codestral_call::error::CodestralCallError;
use crate::codestral_call::execute::CodestralCall;
use crate::codestral_call::plan::{Mode, Plan, Wrapper};
use crate::codestral_records::RecordSource;
use crate::codestral_records::chunk::CodestralChunk;
use crate::codestral_records::error::TextCompletionCodestralError;
use crate::openai_base_config::Headers;

const REPLY: &[u8] =
    br#"{"id":"cmpl-1","created":1700000000,"choices":[{"index":0,"message":{"content":"hi"},"finish_reason":"stop"}]}"#;
const STREAM_BODY: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n";
const THREE_LINES: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n";

struct Captured {
    request_line: String,
    headers: BTreeMap<String, String>,
    header_lines: Vec<(String, String)>,
    body: Value,
}

async fn read_request(stream: &mut TcpStream) -> Captured {
    let mut raw = Vec::new();
    loop {
        if raw.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await.unwrap();
        raw.push(byte[0]);
    }
    let head = String::from_utf8(raw).unwrap();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let header_lines: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let headers: BTreeMap<String, String> = header_lines.iter().cloned().collect();
    let length: usize = headers
        .get("content-length")
        .map_or(0, |value| value.parse().unwrap());
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    Captured {
        request_line,
        headers,
        header_lines,
        body: serde_json::from_slice(&body).unwrap(),
    }
}

async fn respond(stream: &mut TcpStream, status: u16, answer: &[u8]) -> bool {
    let head = format!(
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        answer.len()
    );
    stream.write_all(head.as_bytes()).await.is_ok()
        && stream.write_all(answer).await.is_ok()
        && stream.shutdown().await.is_ok()
}

async fn serve_once(listener: TcpListener, status: u16, answer: &[u8]) -> Captured {
    let (mut stream, _) = listener.accept().await.unwrap();
    let captured = read_request(&mut stream).await;
    respond(&mut stream, status, answer).await;
    captured
}

async fn serve_delayed(listener: TcpListener, delay: Duration, answer: &[u8]) -> Captured {
    let (mut stream, _) = listener.accept().await.unwrap();
    let captured = read_request(&mut stream).await;
    sleep(delay).await;
    respond(&mut stream, 200, answer).await;
    captured
}

async fn serve_until_closed(listener: TcpListener) -> bool {
    let (mut socket, _) = listener.accept().await.unwrap();
    read_request(&mut socket).await;
    let head =
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
    let line = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n";
    let chunk = format!("{:x}\r\n", line.len());
    if socket.write_all(head.as_bytes()).await.is_err()
        || socket.write_all(chunk.as_bytes()).await.is_err()
        || socket.write_all(line).await.is_err()
        || socket.write_all(b"\r\n").await.is_err()
    {
        return false;
    }
    let mut buffer = [0_u8; 1];
    let closed = timeout(Duration::from_secs(2), socket.read(&mut buffer)).await;
    matches!(closed, Ok(Ok(0) | Err(_)))
}

fn new_call() -> CodestralCall {
    new_call_with(TransportLimits::DEFAULT)
}

fn new_call_with(limits: TransportLimits) -> CodestralCall {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    CodestralCall {
        client: ProxiedClient::build(base_builder(), &policy, limits).unwrap(),
    }
}

fn plan_for(url: &str, mode: Mode, streaming: bool, timeout: Option<Duration>) -> Plan {
    let headers = Headers::from([
        ("content-type".to_owned(), "application/json".to_owned()),
        ("Authorization".to_owned(), "Bearer sk-key".to_owned()),
    ]);
    let body = Map::from_iter([
        ("model".to_owned(), Value::from("codestral-latest")),
        ("prompt".to_owned(), Value::from("write")),
    ]);
    Plan {
        mode,
        headers,
        url: url.to_owned(),
        body,
        wrapper: streaming.then_some(match mode {
            Mode::Sync => Wrapper::Codestral,
            Mode::Async => Wrapper::TextCompletionCodestral,
        }),
        timeout,
        prompt: "write".to_owned(),
        messages: Vec::new(),
        api_key: SecretString::from("sk-key"),
    }
}

fn source() -> RecordSource {
    RecordSource {
        id: Uuid::nil(),
        created: 0,
    }
}

async fn collect_chunks<S>(source: S) -> Vec<CodestralChunk>
where
    S: Stream<Item = Result<CodestralChunk, CodestralCallError>>,
{
    let mut stream = Box::pin(source);
    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        chunks.push(item.unwrap());
    }
    chunks
}

async fn first_then_drop<S>(source: S)
where
    S: Stream<Item = Result<CodestralChunk, CodestralCallError>>,
{
    let mut stream = Box::pin(source);
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.text, "hi");
}

#[tokio::test]
async fn complete_sends_the_planned_request_and_decodes_the_reply() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(
        &format!("http://{address}/fim"),
        Mode::Async,
        false,
        Some(Duration::from_secs(5)),
    );
    let mut recorder = Recorder::default();
    let (outcome, captured) = tokio::join!(
        call.complete(&plan, source(), &mut recorder),
        serve_once(listener, 200, REPLY)
    );
    let reply = outcome.unwrap();
    assert_eq!(reply.object, "text_completion");
    assert_eq!(reply.choices[0].text.as_deref(), Some("hi"));
    assert_eq!(captured.request_line, "POST /fim HTTP/1.1");
    assert_eq!(
        captured.headers.get("authorization").map(String::as_str),
        Some("Bearer sk-key")
    );
    assert_eq!(
        captured.body.get("prompt").and_then(Value::as_str),
        Some("write")
    );
    assert_eq!(recorder.phases(), vec!["post_call"]);
}

#[tokio::test]
async fn a_case_different_caller_header_keeps_its_own_line() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let mut plan = plan_for(
        &format!("http://{address}/fim"),
        Mode::Async,
        false,
        Some(Duration::from_secs(5)),
    );
    plan.headers
        .insert("Content-Type".to_owned(), "text/plain".to_owned());
    let mut recorder = Recorder::default();
    let (outcome, captured) = tokio::join!(
        call.complete(&plan, source(), &mut recorder),
        serve_once(listener, 200, REPLY)
    );
    outcome.unwrap();
    let mut values: Vec<&str> = captured
        .header_lines
        .iter()
        .filter(|(name, _)| name == "content-type")
        .map(|(_, value)| value.as_str())
        .collect();
    values.sort_unstable();
    assert_eq!(values, vec!["application/json", "text/plain"]);
}

#[tokio::test]
async fn a_client_error_reply_fails_before_the_post_call() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(
        &format!("http://{address}/fim"),
        Mode::Async,
        false,
        Some(Duration::from_secs(5)),
    );
    let mut recorder = Recorder::default();
    let (outcome, _captured) = tokio::join!(
        call.complete(&plan, source(), &mut recorder),
        serve_once(listener, 400, b"bad request")
    );
    let error = outcome.unwrap_err();
    assert_eq!(error.status_code(), Some(400));
    let CodestralCallError::Record(TextCompletionCodestralError::Native(native)) = &error else {
        panic!("a rejected reply keeps its native status")
    };
    assert_eq!(native.status_code, 400);
    assert!(recorder.phases().is_empty());
}

#[tokio::test]
async fn a_two_hundred_and_one_reply_logs_the_post_call_before_its_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(
        &format!("http://{address}/fim"),
        Mode::Async,
        false,
        Some(Duration::from_secs(5)),
    );
    let mut recorder = Recorder::default();
    let (outcome, _captured) = tokio::join!(
        call.complete(&plan, source(), &mut recorder),
        serve_once(listener, 201, REPLY)
    );
    let error = outcome.unwrap_err();
    let CodestralCallError::Record(TextCompletionCodestralError::Native(native)) = &error else {
        panic!("a 201 reply keeps its native status")
    };
    assert_eq!(native.status_code, 201);
    assert_eq!(recorder.phases(), vec!["post_call"]);
}

#[tokio::test]
async fn a_transport_failure_maps_to_status_500() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let call = new_call();
    let plan = plan_for(
        &format!("http://{address}/fim"),
        Mode::Async,
        false,
        Some(Duration::from_secs(5)),
    );
    let mut recorder = Recorder::default();
    let error = call
        .complete(&plan, source(), &mut recorder)
        .await
        .unwrap_err();
    assert!(matches!(error, CodestralCallError::Transport { .. }));
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn a_stream_sends_only_when_it_is_polled() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Async, true, None);
    let planned = call
        .stream(plan, source(), Box::new(Recorder::default()))
        .unwrap();
    let mut stream = Box::pin(planned);
    let idle = timeout(Duration::from_millis(100), listener.accept()).await;
    idle.unwrap_err();
    let (first, captured) = tokio::join!(stream.next(), serve_once(listener, 200, STREAM_BODY));
    assert_eq!(captured.request_line, "POST /fim HTTP/1.1");
    assert_eq!(first.unwrap().unwrap().text, "a");
}

#[tokio::test]
async fn a_stream_status_other_than_200_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Async, true, None);
    let recorder = Recorder::default();
    let planned = call
        .stream(plan, source(), Box::new(recorder.clone()))
        .unwrap();
    let mut stream = Box::pin(planned);
    let (first, _captured) = tokio::join!(stream.next(), serve_once(listener, 201, STREAM_BODY));
    let error = first.unwrap().unwrap_err();
    let CodestralCallError::Record(TextCompletionCodestralError::Native(native)) = &error else {
        panic!("a non-200 stream keeps its native status")
    };
    assert_eq!(native.status_code, 201);
    assert!(recorder.phases().is_empty());
}

#[tokio::test]
async fn the_async_stream_logs_the_lines_with_an_empty_key() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Async, true, None);
    let recorder = Recorder::default();
    let planned = call
        .stream(plan, source(), Box::new(recorder.clone()))
        .unwrap();
    let mut stream = Box::pin(planned);
    let (first, _captured) = tokio::join!(stream.next(), serve_once(listener, 200, STREAM_BODY));
    first.unwrap().unwrap();
    let events = recorder.events();
    let Event::StreamPost(api_key, request_data) = &events[0] else {
        panic!("the stream logs the line stream first")
    };
    assert_eq!(api_key, "");
    let logged: Map<String, Value> = serde_json::from_str(request_data).unwrap();
    assert_eq!(
        logged.get("model").and_then(Value::as_str),
        Some("codestral-latest")
    );
}

#[tokio::test]
async fn a_sync_stream_logs_no_stream_post_call() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Sync, true, None);
    let recorder = Recorder::default();
    let planned = call
        .stream(plan, source(), Box::new(recorder.clone()))
        .unwrap();
    let mut stream = Box::pin(planned);
    let (first, _captured) = tokio::join!(stream.next(), serve_once(listener, 200, STREAM_BODY));
    first.unwrap().unwrap();
    assert!(recorder.phases().is_empty());
}

#[tokio::test]
async fn the_stream_decodes_every_line() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Async, true, None);
    let planned = call
        .stream(plan, source(), Box::new(Recorder::default()))
        .unwrap();
    let (chunks, _captured) = tokio::join!(
        collect_chunks(planned),
        serve_once(listener, 200, THREE_LINES)
    );
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].text, "a");
    assert!(!chunks[0].is_finished);
    assert_eq!(chunks[1].text, "");
    assert!(!chunks[1].is_finished);
    assert!(chunks[2].is_finished);
    assert_eq!(chunks[2].finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn cancelling_the_stream_closes_the_upstream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Async, true, None);
    let planned = call
        .stream(plan, source(), Box::new(Recorder::default()))
        .unwrap();
    let (closed, ()) = tokio::join!(serve_until_closed(listener), first_then_drop(planned));
    assert!(closed);
}

#[tokio::test]
async fn a_sync_request_uses_the_shared_client_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut call = new_call();
    call.client.limits.read_timeout = Duration::from_millis(50);
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Sync, false, None);
    let mut recorder = Recorder::default();
    let (outcome, _captured) = tokio::join!(
        call.complete(&plan, source(), &mut recorder),
        serve_delayed(listener, Duration::from_millis(300), b"{}")
    );
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        CodestralCallError::Transport {
            source: TransportError::Deadline(_),
            ..
        }
    ));
    assert_eq!(error.status_code(), Some(408));
}

#[tokio::test]
async fn an_async_reply_uses_the_supplied_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut call = new_call();
    call.client.limits.read_timeout = Duration::from_millis(50);
    let plan = plan_for(
        &format!("http://{address}/fim"),
        Mode::Async,
        false,
        Some(Duration::from_secs(5)),
    );
    let mut recorder = Recorder::default();
    let (outcome, _captured) = tokio::join!(
        call.complete(&plan, source(), &mut recorder),
        serve_delayed(listener, Duration::from_millis(150), b"{}")
    );
    outcome.unwrap();
}

#[tokio::test]
async fn a_sync_stream_uses_the_shared_client_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut call = new_call();
    call.client.limits.read_timeout = Duration::from_millis(50);
    let plan = plan_for(&format!("http://{address}/fim"), Mode::Sync, true, None);
    let planned = call
        .stream(plan, source(), Box::new(Recorder::default()))
        .unwrap();
    let mut stream = Box::pin(planned);
    let (first, _captured) = tokio::join!(
        stream.next(),
        serve_delayed(listener, Duration::from_millis(300), STREAM_BODY)
    );
    let error = first.unwrap().unwrap_err();
    assert!(matches!(
        error,
        CodestralCallError::Transport {
            source: TransportError::Deadline(_),
            ..
        }
    ));
    assert_eq!(error.status_code(), Some(408));
}

#[tokio::test]
async fn a_connect_timeout_reports_408() {
    let mut limits = TransportLimits::DEFAULT;
    limits.connect_timeout = Duration::from_millis(100);
    let call = new_call_with(limits);
    let plan = plan_for("http://192.0.2.1:9/fim", Mode::Sync, false, None);
    let mut recorder = Recorder::default();
    let error = call
        .complete(&plan, source(), &mut recorder)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CodestralCallError::Transport {
            source: TransportError::ConnectTimeout(_),
            ..
        }
    ));
    assert_eq!(error.status_code(), Some(408));
}

#[tokio::test]
async fn an_async_reply_timeout_maps_to_status_500() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let call = new_call();
    let plan = plan_for(
        &format!("http://{address}/fim"),
        Mode::Async,
        false,
        Some(Duration::from_millis(50)),
    );
    let mut recorder = Recorder::default();
    let (outcome, _captured) = tokio::join!(
        call.complete(&plan, source(), &mut recorder),
        serve_delayed(listener, Duration::from_millis(300), b"{}")
    );
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        CodestralCallError::Transport {
            source: TransportError::Deadline(_),
            ..
        }
    ));
    assert_eq!(error.status_code(), Some(500));
}
