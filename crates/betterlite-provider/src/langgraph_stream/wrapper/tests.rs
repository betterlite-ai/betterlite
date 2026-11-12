#![cfg(test)]

use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::client::ClientKind;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use uuid::Uuid;

use super::{
    LangGraphStream, StreamDeps, StreamHead, StreamLogger, StreamPostCall, StreamRequest,
    StreamTransport, open,
};
use crate::langgraph_chat::error::LangGraphError;
use crate::langgraph_chat::records::{
    LangGraphInput, LangGraphMessage, LangGraphRequest, LangGraphRole,
};
use crate::langgraph_stream::error::LangGraphStreamError;
use crate::langgraph_stream::{FIRST_STREAM_RESPONSE, IdSource, MAX_BODY_BYTES};

const URL: &str = "https://agents.example/runs/stream";
const MODEL: &str = "langgraph/agent";

struct FixedIds {
    next: Mutex<u32>,
}

impl FixedIds {
    fn new() -> Self {
        Self {
            next: Mutex::new(0),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut next = self.next.lock().unwrap();
        let id = Uuid::from_u128(u128::from(*next));
        *next = next.checked_add(1).unwrap();
        id
    }
}

struct Seen {
    method: String,
    url: String,
    headers: HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
}

impl Clone for Seen {
    fn clone(&self) -> Self {
        Self {
            method: self.method.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            body: self.body.clone(),
            deadline: self.deadline,
        }
    }
}

struct FakeTransport {
    kind: ClientKind,
    status: u16,
    frames: Mutex<Vec<Result<Vec<u8>, HttpError>>>,
    seen: Mutex<Vec<Seen>>,
    closed: bool,
}

impl FakeTransport {
    fn new(kind: ClientKind, status: u16, frames: Vec<Result<Vec<u8>, HttpError>>) -> Self {
        Self {
            kind,
            status,
            frames: Mutex::new(frames),
            seen: Mutex::new(Vec::new()),
            closed: false,
        }
    }

    fn closed(kind: ClientKind) -> Self {
        Self {
            closed: true,
            ..Self::new(kind, 200, Vec::new())
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl StreamTransport for FakeTransport {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    async fn post(&self, request: OutboundRequest) -> Result<StreamHead, TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let RequestBody::Json(body) = request.body else {
            panic!("the LangGraph wrapper sends a JSON body");
        };
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: request.url.to_string(),
            headers: request.headers,
            body,
            deadline: request.deadline,
        });
        let frames = mem::take(&mut *self.frames.lock().unwrap());
        Ok(StreamHead {
            status: self.status,
            chunks: stream::iter(frames).boxed(),
        })
    }
}

#[derive(Default)]
struct RecordingLogger {
    calls: Mutex<Vec<Recorded>>,
}

#[derive(Clone)]
struct Recorded {
    input: Vec<Message>,
    api_key: String,
    original_response: String,
    complete_input_dict: LangGraphRequest,
}

impl RecordingLogger {
    fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

impl StreamLogger for RecordingLogger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        self.calls.lock().unwrap().push(Recorded {
            input: call.input.to_vec(),
            api_key: call.api_key.to_owned(),
            original_response: call.original_response.to_owned(),
            complete_input_dict: call.complete_input_dict.clone(),
        });
    }
}

fn body() -> LangGraphRequest {
    LangGraphRequest {
        assistant_id: String::from("agent"),
        input: LangGraphInput {
            messages: vec![LangGraphMessage {
                role: LangGraphRole::Human,
                content: String::from("hi"),
                metadata: None,
            }],
        },
        stream_mode: Some(Json::Text(String::from("messages-tuple"))),
        config: None,
        metadata: None,
        thread_id: None,
    }
}

fn prompt() -> Vec<Message> {
    vec![Message {
        role: String::from("user"),
        content: Some(Content::Text(String::from("hi"))),
        name: None,
        provider_specific_fields: Object::new(),
    }]
}

fn request<'request>(
    body: &'request LangGraphRequest,
    messages: &'request [Message],
    kind: ClientKind,
) -> StreamRequest<'request> {
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", HeaderValue::from_static("Bearer secret"));
    StreamRequest {
        model: MODEL,
        provider: "langgraph",
        url: URL,
        headers,
        data: body,
        messages,
        kind,
    }
}

fn deps<'deps>(
    supplied: Option<Arc<FakeTransport>>,
    default: &Arc<FakeTransport>,
    logger: &'deps RecordingLogger,
) -> StreamDeps<'deps> {
    let supplied_client: Option<Arc<dyn StreamTransport>> = supplied.map(|transport| {
        let client: Arc<dyn StreamTransport> = transport;
        client
    });
    let cloned: Arc<FakeTransport> = Arc::clone(default);
    let default_client: Arc<dyn StreamTransport> = cloned;
    StreamDeps {
        supplied: supplied_client,
        default_client,
        ids: Arc::new(FixedIds::new()),
        logger,
    }
}

fn frames(body: &str) -> Vec<Result<Vec<u8>, HttpError>> {
    vec![Ok(body.as_bytes().to_vec())]
}

fn expect_error<T>(result: Result<T, LangGraphStreamError>, message: &str) -> LangGraphStreamError {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn posts_the_serialized_body_with_the_headers_and_the_deadline() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200, frames("")));
    let logger = RecordingLogger::default();
    let stream = open(
        &request(&data, &messages, ClientKind::Async),
        &deps(None, &transport, &logger),
    )
    .await
    .unwrap();
    assert_eq!(stream.model, MODEL);
    assert_eq!(stream.provider, "langgraph");
    let seen = transport.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, URL);
    assert_eq!(seen[0].headers["Authorization"], "Bearer secret");
    assert_eq!(seen[0].deadline, TransportLimits::DEFAULT.read_timeout);
    let sent: LangGraphRequest = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(sent, data);
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::redirect(301)]
#[case::bad_request(400)]
#[case::server_error(500)]
#[tokio::test]
async fn rejects_every_status_except_200(#[case] status: u16) {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Async,
        status,
        frames("refused"),
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &request(&data, &messages, ClientKind::Async),
            &deps(None, &transport, &logger),
        )
        .await,
        "the rejected status was accepted",
    );
    let LangGraphStreamError::Provider(failure) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(failure.status_code, status);
    assert_eq!(failure.message, "refused");
}

#[tokio::test]
async fn the_error_message_is_the_lossy_body_text() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Async,
        502,
        vec![Ok(vec![0xff, b'a'])],
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &request(&data, &messages, ClientKind::Async),
            &deps(None, &transport, &logger),
        )
        .await,
        "the failed request was accepted",
    );
    let LangGraphStreamError::Provider(LangGraphError {
        status_code,
        message,
    }) = error
    else {
        panic!("expected a provider error");
    };
    assert_eq!(status_code, 502);
    assert_eq!(message, "\u{fffd}a");
}

#[tokio::test]
async fn an_error_body_over_the_byte_limit_is_refused() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Async,
        500,
        vec![Ok(vec![b'x'; MAX_BODY_BYTES + 1])],
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &request(&data, &messages, ClientKind::Async),
            &deps(None, &transport, &logger),
        )
        .await,
        "the oversized error body was accepted",
    );
    assert!(matches!(error, LangGraphStreamError::TooLarge { limit } if limit == MAX_BODY_BYTES));
}

#[tokio::test]
async fn an_error_body_of_exactly_the_limit_is_read() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Async,
        500,
        vec![Ok(vec![b'x'; MAX_BODY_BYTES])],
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &request(&data, &messages, ClientKind::Async),
            &deps(None, &transport, &logger),
        )
        .await,
        "the at-limit error body was refused",
    );
    let LangGraphStreamError::Provider(failure) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(failure.message.len(), MAX_BODY_BYTES);
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
    let data = body();
    let messages = prompt();
    let supplied = Arc::new(FakeTransport::new(supplied_kind, 200, frames("")));
    let default = Arc::new(FakeTransport::new(call_kind, 200, frames("")));
    let logger = RecordingLogger::default();
    let stream = open(
        &request(&data, &messages, call_kind),
        &deps(Some(Arc::clone(&supplied)), &default, &logger),
    )
    .await
    .unwrap();
    drop(stream);
    assert_eq!(supplied.seen().len(), usize::from(kept));
    assert_eq!(default.seen().len(), usize::from(!kept));
}

#[tokio::test]
async fn opens_on_the_default_client_without_a_supplied_one() {
    let data = body();
    let messages = prompt();
    let default = Arc::new(FakeTransport::new(ClientKind::Sync, 200, frames("")));
    let logger = RecordingLogger::default();
    let stream = open(
        &request(&data, &messages, ClientKind::Sync),
        &deps(None, &default, &logger),
    )
    .await
    .unwrap();
    drop(stream);
    assert_eq!(default.seen().len(), 1);
}

#[tokio::test]
async fn logs_post_call_after_a_200() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200, frames("")));
    let logger = RecordingLogger::default();
    let stream = open(
        &request(&data, &messages, ClientKind::Async),
        &deps(None, &transport, &logger),
    )
    .await
    .unwrap();
    drop(stream);
    let calls = logger.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input, messages);
    assert_eq!(calls[0].api_key, "");
    assert_eq!(calls[0].original_response, FIRST_STREAM_RESPONSE);
    assert_eq!(calls[0].complete_input_dict, data);
}

#[tokio::test]
async fn does_not_log_post_call_when_the_status_is_not_200() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Async,
        500,
        frames("refused"),
    ));
    let logger = RecordingLogger::default();
    let result = open(
        &request(&data, &messages, ClientKind::Async),
        &deps(None, &transport, &logger),
    )
    .await;
    assert!(result.is_err());
    assert!(logger.calls().is_empty());
}

#[tokio::test]
async fn the_wrapper_parses_the_response_stream() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Async,
        200,
        frames("data: {\"content\": \"hi\"}\n\ndata: [\"metadata\", {\"run_id\": \"r\"}]\n\n"),
    ));
    let logger = RecordingLogger::default();
    let stream: LangGraphStream = open(
        &request(&data, &messages, ClientKind::Async),
        &deps(None, &transport, &logger),
    )
    .await
    .unwrap();
    let mut iterator = stream.iterator;
    let first = iterator.next_chunk().await.unwrap();
    assert_eq!(first.choices[0].delta.content.as_deref(), Some("hi"));
    let last = iterator.next_chunk().await.unwrap();
    assert_eq!(last.choices[0].finish_reason.as_deref(), Some("stop"));
    assert!(iterator.next_chunk().await.is_none());
}

#[tokio::test]
async fn a_transport_error_surfaces() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::closed(ClientKind::Async));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &request(&data, &messages, ClientKind::Async),
            &deps(None, &transport, &logger),
        )
        .await,
        "the closed transport answered",
    );
    assert!(matches!(
        error,
        LangGraphStreamError::Transport {
            source: TransportError::Closed
        }
    ));
}

#[tokio::test]
async fn an_invalid_url_is_refused() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(ClientKind::Async, 200, frames("")));
    let logger = RecordingLogger::default();
    let mut request = request(&data, &messages, ClientKind::Async);
    request.url = "::not a url::";
    let error = expect_error(
        open(&request, &deps(None, &transport, &logger)).await,
        "the invalid URL was accepted",
    );
    assert!(matches!(error, LangGraphStreamError::Url { .. }));
}
