#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::bytez_wire::messages::RawMessage;
use betterlite_openai::bytez_wire::request::request_body;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::StreamShapeError;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use tokio::sync::mpsc;

use super::BytezStreamCall;
use super::error::BytezStreamError;
use super::stream::BytezCustomStream;
use super::transport::{BytezStreamTransport, ClientError, StreamHead};

const URL: &str = "https://api.bytez.com/models/v2/org/model";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    method: String,
    url: String,
    headers: HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
}

struct FakeTransport {
    kind: ClientKind,
    status: u16,
    failure: Option<(u16, String)>,
    queue: Mutex<Option<mpsc::Sender<Result<Vec<u8>, HttpError>>>>,
    receiver: Mutex<Option<mpsc::Receiver<Result<Vec<u8>, HttpError>>>>,
    seen: Mutex<Vec<Seen>>,
    dropped: Arc<AtomicBool>,
}

impl FakeTransport {
    fn new(kind: ClientKind, status: u16) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        Self {
            kind,
            status,
            failure: None,
            queue: Mutex::new(Some(sender)),
            receiver: Mutex::new(Some(receiver)),
            seen: Mutex::new(Vec::new()),
            dropped: Arc::new(AtomicBool::new(false)),
        }
    }

    fn failing(kind: ClientKind, status: u16, message: &str) -> Self {
        Self {
            failure: Some((status, message.to_owned())),
            ..Self::new(kind, status)
        }
    }

    fn push(&self, chunk: &[u8]) {
        self.queue
            .lock()
            .unwrap()
            .as_ref()
            .expect("queue")
            .try_send(Ok(chunk.to_vec()))
            .expect("queued fragment");
    }

    fn fail_with(&self, error: HttpError) {
        self.queue
            .lock()
            .unwrap()
            .as_ref()
            .expect("queue")
            .try_send(Err(error))
            .expect("queued failure");
    }

    fn close(&self) {
        self.queue.lock().unwrap().take();
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl BytezStreamTransport for FakeTransport {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    async fn open(&self, request: OutboundRequest) -> Result<StreamHead, ClientError> {
        let body = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        self.seen.lock().unwrap().push(Seen {
            method: request.method.to_string(),
            url: request.url.to_string(),
            headers: request.headers,
            body,
            deadline: request.deadline,
        });
        if let Some((status_code, message)) = self.failure.clone() {
            return Err(ClientError::Status {
                status_code,
                message,
            });
        }
        let receiver = self.receiver.lock().unwrap().take().expect("receiver");
        let guard = DropFlag(Arc::clone(&self.dropped));
        let chunks = stream::unfold((receiver, guard), |(mut receiver, guard)| async move {
            receiver.recv().await.map(|item| (item, (receiver, guard)))
        });
        Ok(StreamHead {
            status: self.status,
            chunks: chunks.boxed(),
        })
    }
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor")
}

fn message() -> RawMessage {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("message")
}

fn params() -> Object {
    match serde_json::from_str(r#"{"stream":true,"max_new_tokens":10}"#).expect("params") {
        Json::Object(fields) => fields,
        other => panic!("{other:?} is not an object"),
    }
}

fn call<'request>(
    supplied: Option<Arc<FakeTransport>>,
    default: Arc<FakeTransport>,
    kind: ClientKind,
    redactor: &'request Redactor,
) -> BytezStreamCall<'request> {
    let supplied: Option<Arc<dyn BytezStreamTransport>> = supplied.map(|transport| {
        let shared: Arc<dyn BytezStreamTransport> = transport;
        shared
    });
    let default: Arc<dyn BytezStreamTransport> = default;
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", HeaderValue::from_static("Key secret"));
    BytezStreamCall {
        model: String::from("org/model"),
        provider: String::from("bytez"),
        url: String::from(URL),
        headers,
        body: request_body(&[message()], &params()).expect("body"),
        kind,
        supplied,
        default_client: default,
        repeated_chunk_limit: 100,
        redactor,
    }
}

async fn opened(
    supplied: Option<Arc<FakeTransport>>,
    kind: ClientKind,
    redactor: &Redactor,
) -> (BytezCustomStream<'_>, Arc<FakeTransport>) {
    let default = Arc::new(FakeTransport::new(kind, 200));
    let stream = call(supplied, Arc::clone(&default), kind, redactor)
        .open()
        .await
        .expect("stream");
    (stream, default)
}

async fn first_fragment(stream: &mut BytezCustomStream<'_>) -> String {
    stream
        .fragments
        .next()
        .await
        .expect("fragment")
        .expect("text")
}

#[tokio::test]
async fn opens_a_post_with_the_serialized_body_and_the_stream_timeout() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200));
    let (stream, _default) =
        opened(Some(Arc::clone(&transport)), ClientKind::Async, &redactor).await;
    drop(stream);
    let seen = transport.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, URL);
    assert_eq!(seen[0].headers["Authorization"], "Key secret");
    assert_eq!(seen[0].deadline, super::STREAMING_TIMEOUT);
    assert_eq!(seen[0].deadline, Duration::from_secs(300));
    let body: serde_json::Value = serde_json::from_slice(&seen[0].body).expect("body");
    assert_eq!(body["stream"], serde_json::Value::Bool(true));
    assert_eq!(
        body["params"]["max_new_tokens"],
        serde_json::Value::from(10)
    );
    assert!(body["params"].get("stream").is_none());
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::redirect(301)]
#[case::bad_request(400)]
#[case::server_error(500)]
#[tokio::test]
async fn rejects_every_status_except_200(#[case] status: u16) {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, status));
    transport.push(b"refused body");
    let Err(error) = call(
        Some(Arc::clone(&transport)),
        Arc::new(FakeTransport::new(ClientKind::Async, 200)),
        ClientKind::Async,
        &redactor,
    )
    .open()
    .await
    else {
        panic!("opened");
    };
    match error {
        BytezStreamError::Api(error) => {
            assert_eq!(error.status_code, status);
            assert_eq!(error.message, "refused body");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn wraps_a_transport_status_error() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::failing(
        ClientKind::Async,
        503,
        "upstream down",
    ));
    let Err(error) = call(
        Some(Arc::clone(&transport)),
        Arc::new(FakeTransport::new(ClientKind::Async, 200)),
        ClientKind::Async,
        &redactor,
    )
    .open()
    .await
    else {
        panic!("opened");
    };
    match error {
        BytezStreamError::Api(error) => {
            assert_eq!(error.status_code, 503);
            assert_eq!(error.message, "upstream down");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn refuses_a_refusal_body_over_the_byte_limit() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 500));
    transport.push(&vec![b'x'; super::MAX_BODY_BYTES + 1]);
    let Err(error) = call(
        Some(Arc::clone(&transport)),
        Arc::new(FakeTransport::new(ClientKind::Async, 200)),
        ClientKind::Async,
        &redactor,
    )
    .open()
    .await
    else {
        panic!("opened");
    };
    assert!(
        matches!(error, BytezStreamError::TooLarge { limit } if limit == super::MAX_BODY_BYTES)
    );
}

#[rstest]
#[case::sync_keeps_sync(ClientKind::Sync, ClientKind::Sync, true)]
#[case::async_keeps_async(ClientKind::Async, ClientKind::Async, true)]
#[case::sync_replaces_async(ClientKind::Sync, ClientKind::Async, false)]
#[case::async_replaces_sync(ClientKind::Async, ClientKind::Sync, false)]
#[tokio::test]
async fn uses_a_supplied_client_only_when_its_kind_matches(
    #[case] call_kind: ClientKind,
    #[case] supplied_kind: ClientKind,
    #[case] kept: bool,
) {
    let redactor = redactor();
    let supplied = Arc::new(FakeTransport::new(supplied_kind, 200));
    let default = Arc::new(FakeTransport::new(call_kind, 200));
    let stream = call(
        Some(Arc::clone(&supplied)),
        Arc::clone(&default),
        call_kind,
        &redactor,
    )
    .open()
    .await
    .expect("stream");
    drop(stream);
    assert_eq!(supplied.seen().len(), usize::from(kept));
    assert_eq!(default.seen().len(), usize::from(!kept));
}

#[tokio::test]
async fn opens_without_a_supplied_client_on_the_default() {
    let redactor = redactor();
    let default = Arc::new(FakeTransport::new(ClientKind::Sync, 200));
    let stream = call(None, Arc::clone(&default), ClientKind::Sync, &redactor)
        .open()
        .await
        .expect("stream");
    drop(stream);
    assert_eq!(default.seen().len(), 1);
}

#[tokio::test]
async fn delivers_each_fragment_before_the_next_one_arrives() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200));
    transport.push(b"one");
    let (mut stream, _default) =
        opened(Some(Arc::clone(&transport)), ClientKind::Async, &redactor).await;
    assert_eq!(first_fragment(&mut stream).await, "one");
    let pending = tokio::time::timeout(Duration::from_millis(20), stream.fragments.next()).await;
    assert!(pending.is_err());
    transport.push(b"two");
    assert_eq!(first_fragment(&mut stream).await, "two");
}

#[tokio::test]
async fn keeps_a_split_character_for_the_next_fragment() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200));
    transport.push(&[0xc3]);
    transport.push(&[0xa9]);
    let (mut stream, _default) =
        opened(Some(Arc::clone(&transport)), ClientKind::Async, &redactor).await;
    assert_eq!(first_fragment(&mut stream).await, "é");
}

#[tokio::test]
async fn refuses_bytes_that_are_not_utf8() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200));
    transport.push(&[0xff]);
    let (mut stream, _default) =
        opened(Some(Arc::clone(&transport)), ClientKind::Async, &redactor).await;
    let error = stream
        .fragments
        .next()
        .await
        .expect("fragment")
        .expect_err("refused");
    assert!(matches!(error, BytezStreamError::InvalidUtf8 { .. }));
}

#[tokio::test]
async fn delivers_embedded_newlines_and_data_text_as_plain_content() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200));
    transport.push(b"line one\ndata: not an event");
    let (mut stream, _default) =
        opened(Some(Arc::clone(&transport)), ClientKind::Async, &redactor).await;
    assert_eq!(
        first_fragment(&mut stream).await,
        "line one\ndata: not an event"
    );
}

#[tokio::test]
async fn a_body_error_surfaces_as_a_body_failure() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200));
    transport.fail_with(HttpError::Timeout);
    let (mut stream, _default) =
        opened(Some(Arc::clone(&transport)), ClientKind::Async, &redactor).await;
    let error = stream
        .fragments
        .next()
        .await
        .expect("fragment")
        .expect_err("failed");
    assert!(matches!(
        error,
        BytezStreamError::Body {
            source: HttpError::Timeout
        }
    ));
}

#[tokio::test]
async fn dropping_the_stream_drops_the_transport_body() {
    let redactor = redactor();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200));
    let (stream, _default) =
        opened(Some(Arc::clone(&transport)), ClientKind::Async, &redactor).await;
    assert!(!transport.dropped.load(Ordering::SeqCst));
    drop(stream);
    assert!(transport.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn chunk_creator_shapes_a_content_chunk() {
    let redactor = redactor();
    let (mut stream, _default) = opened(None, ClientKind::Sync, &redactor).await;
    let chunk = stream
        .chunk_creator("hello")
        .expect("chunk")
        .expect("emitted");
    assert_eq!(chunk.object, "chat.completion.chunk");
    assert_eq!(chunk.model, "org/model");
    assert!(chunk.id.starts_with("chatcmpl-"));
    assert_eq!(chunk.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hello"));
    assert_eq!(chunk.choices[0].finish_reason, None);
}

#[tokio::test]
async fn chunk_creator_suppresses_an_empty_fragment() {
    let redactor = redactor();
    let (mut stream, _default) = opened(None, ClientKind::Sync, &redactor).await;
    assert_eq!(stream.chunk_creator("").expect("suppressed"), None);
}

#[test]
fn preserves_stop_iteration() {
    let redactor = redactor();
    let error = BytezStreamError::from_chunk(
        &ProviderChunkError::StopIteration,
        "org/model",
        "bytez",
        &redactor,
    );
    assert!(matches!(error, BytezStreamError::StopIteration));
}

#[test]
fn maps_other_chunk_failures_through_the_error_classifier() {
    let redactor = redactor();
    let repetition = StreamShapeError::Repetition {
        last_content: String::from("same"),
        model: String::new(),
        provider: String::new(),
    };
    let error = BytezStreamError::from_chunk(
        ProviderChunkError::Shape { source: repetition },
        "org/model",
        "bytez",
        &redactor,
    );
    match error {
        BytezStreamError::Chunk(failure) => {
            assert_eq!(failure.status_code, 500);
            assert_eq!(failure.error_type, "internal_server_error");
            assert_eq!(failure.param, None);
            assert_eq!(
                failure.message,
                "The model is repeating the same chunk = same."
            );
        }
        other => panic!("{other:?}"),
    }
    let field = BytezStreamError::from_chunk(
        &ProviderChunkError::MissingField { field: "role" },
        "org/model",
        "bytez",
        &redactor,
    );
    match field {
        BytezStreamError::Chunk(failure) => {
            assert_eq!(failure.status_code, 500);
            assert_eq!(failure.error_type, "internal_server_error");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn body_limit_matches_the_transport_default() {
    assert_eq!(super::MAX_BODY_BYTES, TransportLimits::DEFAULT.body_bytes);
}
