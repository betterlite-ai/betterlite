#![cfg(test)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_prompts::message_content::{Content, Message};
use betterlite_vertex::vertex_agent_engine::error::AgentEngineError;
use betterlite_vertex::vertex_agent_engine::records::{AgentQueryInput, AgentQueryPayload};
use betterlite_vertex::vertex_agent_stream::AgentEngineStreamError;
use futures_util::Stream;
use futures_util::StreamExt as _;
use futures_util::stream;
use futures_util::stream::BoxStream;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use super::transport::{Chunks, StreamHead, StreamTransport};
use super::{
    FIRST_STREAM_RESPONSE, IdSource, MAX_BODY_BYTES, MAX_LINE_BYTES, STREAMING_TIMEOUT, StreamDeps,
    StreamLogger, StreamPostCall, StreamRequest, VertexAgentStream, VertexAgentStreamError, open,
};

const URL: &str = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/p/locations/us-central1/reasoningEngines/e:streamQuery";
const MODEL: &str = "agent_engine/e";
const PROVIDER: &str = "vertex_ai";

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

#[derive(Clone)]
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
    frames: Mutex<Option<Chunks>>,
    seen: Mutex<Vec<Seen>>,
}

impl FakeTransport {
    fn new(kind: ClientKind, status: u16, chunks: Chunks) -> Self {
        Self {
            kind,
            status,
            frames: Mutex::new(Some(chunks)),
            seen: Mutex::new(Vec::new()),
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
        let RequestBody::Json(body) = request.body else {
            panic!("the Vertex Agent Engine wrapper sends a JSON body");
        };
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: request.url.to_string(),
            headers: request.headers,
            body,
            deadline: request.deadline,
        });
        let chunks = self.frames.lock().unwrap().take().unwrap();
        Ok(StreamHead {
            status: self.status,
            chunks,
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
    complete_input_dict: AgentQueryPayload,
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

// the stream owns this guard, so the count says when the response behind it was dropped
struct HeldBody {
    dropped: Arc<AtomicUsize>,
    first: Option<Vec<u8>>,
}

impl Drop for HeldBody {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl Stream for HeldBody {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut()
            .first
            .take()
            .map_or_else(|| Poll::Pending, |frame| Poll::Ready(Some(Ok(frame))))
    }
}

fn held(dropped: &Arc<AtomicUsize>, first: Option<Vec<u8>>) -> Chunks {
    let body: BoxStream<'static, Result<Vec<u8>, HttpError>> = Box::pin(HeldBody {
        dropped: Arc::clone(dropped),
        first,
    });
    body
}

fn reads(items: Vec<Result<Vec<u8>, HttpError>>) -> Chunks {
    stream::iter(items).boxed()
}

fn frames(payload: &str) -> Chunks {
    reads(vec![Ok(payload.as_bytes().to_vec())])
}

fn body() -> AgentQueryPayload {
    AgentQueryPayload {
        class_method: "stream_query",
        input: AgentQueryInput {
            message: String::from("hi"),
            user_id: String::from("user-1"),
            session_id: None,
        },
    }
}

fn prompt() -> Vec<Message> {
    vec![Message {
        role: String::from("user"),
        content: Some(Content::Text(String::from("hi"))),
        name: None,
        provider_specific_fields: IndexMap::default(),
    }]
}

fn stream_request<'request>(
    data: &'request AgentQueryPayload,
    messages: &'request [Message],
    kind: ClientKind,
) -> StreamRequest<'request> {
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", HeaderValue::from_static("Bearer secret"));
    StreamRequest {
        model: MODEL,
        provider: PROVIDER,
        url: URL,
        headers,
        data,
        messages,
        kind,
    }
}

fn stream_deps(
    logger: &RecordingLogger,
    supplied: Option<Arc<FakeTransport>>,
    default: Arc<FakeTransport>,
) -> StreamDeps<'_> {
    let supplied_client: Option<Arc<dyn StreamTransport>> = supplied.map(|transport| {
        let client: Arc<dyn StreamTransport> = transport;
        client
    });
    let default_client: Arc<dyn StreamTransport> = default;
    StreamDeps {
        supplied: supplied_client,
        default_client,
        ids: Arc::new(FixedIds::new()),
        logger,
    }
}

fn expect_error<T>(
    result: Result<T, VertexAgentStreamError>,
    message: &str,
) -> VertexAgentStreamError {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

async fn opened(chunks: Chunks) -> VertexAgentStream {
    let data = body();
    let messages = prompt();
    let logger = RecordingLogger::default();
    let transport = Arc::new(FakeTransport::new(ClientKind::Asynchronous, 200, chunks));
    open(
        &stream_request(&data, &messages, ClientKind::Asynchronous),
        &stream_deps(&logger, None, transport),
    )
    .await
    .unwrap()
}

async fn stream_from(items: Vec<Result<Vec<u8>, HttpError>>) -> VertexAgentStream {
    opened(reads(items)).await
}

async fn stream_of(source: &str) -> VertexAgentStream {
    stream_from(vec![Ok(source.as_bytes().to_vec())]).await
}

async fn collect(mut stream: VertexAgentStream) -> Result<Vec<Chunk>, VertexAgentStreamError> {
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next_chunk().await? {
        chunks.push(chunk);
    }
    Ok(chunks)
}

async fn chunk_list(source: &str) -> Result<Vec<Chunk>, VertexAgentStreamError> {
    collect(stream_of(source).await).await
}

fn content_line(text: &str) -> String {
    format!("data: {{\"content\": {{\"parts\": [{{\"text\": \"{text}\"}}]}}}}\n")
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn the_streaming_deadline_is_ten_minutes_and_the_body_cap_is_the_transport_cap() {
    assert_eq!(STREAMING_TIMEOUT, Duration::from_secs(600));
    assert_eq!(MAX_BODY_BYTES, TransportLimits::DEFAULT.body_bytes);
}

#[test]
fn the_default_id_source_generates_a_fresh_uuid() {
    let first = super::UuidV4.next_id();
    let second = super::UuidV4.next_id();
    assert_ne!(first, second);
    assert!(!first.is_nil());
}

#[tokio::test]
async fn posts_the_serialized_request_with_the_headers_and_the_deadline() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Asynchronous,
        200,
        frames(""),
    ));
    let logger = RecordingLogger::default();
    let stream = open(
        &stream_request(&data, &messages, ClientKind::Asynchronous),
        &stream_deps(&logger, None, Arc::clone(&transport)),
    )
    .await
    .unwrap();
    assert_eq!(stream.model, MODEL);
    assert_eq!(stream.provider, PROVIDER);
    let seen = transport.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, URL);
    assert_eq!(seen[0].headers["Authorization"], "Bearer secret");
    assert_eq!(seen[0].deadline, TransportLimits::DEFAULT.read_timeout);
    assert_eq!(seen[0].body, serde_json::to_vec(&data).unwrap());
}

#[rstest]
#[case::sync_keeps_sync(ClientKind::Sync, ClientKind::Sync, true)]
#[case::async_keeps_async(ClientKind::Asynchronous, ClientKind::Asynchronous, true)]
#[case::sync_replaces_async(ClientKind::Sync, ClientKind::Asynchronous, false)]
#[case::async_replaces_sync(ClientKind::Asynchronous, ClientKind::Sync, false)]
#[tokio::test]
async fn sync_async_selection(
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
        &stream_request(&data, &messages, call_kind),
        &stream_deps(&logger, Some(Arc::clone(&supplied)), Arc::clone(&default)),
    )
    .await
    .unwrap();
    drop(stream);
    assert_eq!(supplied.seen().len(), usize::from(kept));
    assert_eq!(default.seen().len(), usize::from(!kept));
}

#[tokio::test]
async fn a_missing_supplied_client_reads_through_the_default() {
    let data = body();
    let messages = prompt();
    let default = Arc::new(FakeTransport::new(ClientKind::Sync, 200, frames("")));
    let logger = RecordingLogger::default();
    let stream = open(
        &stream_request(&data, &messages, ClientKind::Sync),
        &stream_deps(&logger, None, Arc::clone(&default)),
    )
    .await
    .unwrap();
    drop(stream);
    assert_eq!(default.seen().len(), 1);
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::redirect(301)]
#[case::bad_request(400)]
#[case::server_error(500)]
#[tokio::test]
async fn non_200_body(#[case] status: u16) {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Asynchronous,
        status,
        frames("refused"),
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &stream_request(&data, &messages, ClientKind::Asynchronous),
            &stream_deps(&logger, None, Arc::clone(&transport)),
        )
        .await,
        "the rejected status was accepted",
    );
    let VertexAgentStreamError::Provider(AgentEngineError::Api {
        status_code,
        message,
    }) = error
    else {
        panic!("expected a provider error");
    };
    assert_eq!(status_code, status);
    assert_eq!(message, "refused");
}

#[tokio::test]
async fn the_error_message_is_the_lossy_body_text() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Asynchronous,
        502,
        reads(vec![Ok(vec![0xff, b'a'])]),
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &stream_request(&data, &messages, ClientKind::Asynchronous),
            &stream_deps(&logger, None, Arc::clone(&transport)),
        )
        .await,
        "the failed request was accepted",
    );
    let VertexAgentStreamError::Provider(AgentEngineError::Api {
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
        ClientKind::Asynchronous,
        500,
        reads(vec![Ok(vec![b'x'; MAX_BODY_BYTES + 1])]),
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &stream_request(&data, &messages, ClientKind::Asynchronous),
            &stream_deps(&logger, None, Arc::clone(&transport)),
        )
        .await,
        "the oversized error body was accepted",
    );
    assert!(matches!(
        error,
        VertexAgentStreamError::TooLarge { limit } if limit == MAX_BODY_BYTES
    ));
}

#[tokio::test]
async fn an_error_body_of_exactly_the_limit_is_read() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Asynchronous,
        500,
        reads(vec![Ok(vec![b'x'; MAX_BODY_BYTES])]),
    ));
    let logger = RecordingLogger::default();
    let error = expect_error(
        open(
            &stream_request(&data, &messages, ClientKind::Asynchronous),
            &stream_deps(&logger, None, Arc::clone(&transport)),
        )
        .await,
        "the at-limit error body was refused",
    );
    let VertexAgentStreamError::Provider(AgentEngineError::Api { message, .. }) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(message.len(), MAX_BODY_BYTES);
}

#[tokio::test]
async fn first_response_log() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Asynchronous,
        200,
        frames(""),
    ));
    let logger = RecordingLogger::default();
    let stream = open(
        &stream_request(&data, &messages, ClientKind::Asynchronous),
        &stream_deps(&logger, None, Arc::clone(&transport)),
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
async fn no_post_call_is_logged_when_the_status_is_not_200() {
    let data = body();
    let messages = prompt();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Asynchronous,
        500,
        frames("refused"),
    ));
    let logger = RecordingLogger::default();
    let result = open(
        &stream_request(&data, &messages, ClientKind::Asynchronous),
        &stream_deps(&logger, None, Arc::clone(&transport)),
    )
    .await;
    assert!(result.is_err());
    assert!(logger.calls().is_empty());
}

#[tokio::test]
async fn a_content_line_carries_the_litellm_envelope() {
    let chunks = chunk_list(&content_line("hello")).await.unwrap();
    assert_eq!(chunks.len(), 1);
    let chunk = &chunks[0];
    assert_eq!(chunk.id, "chatcmpl-00000000-0000-0000-0000-000000000000");
    assert_eq!(chunk.object, "chat.completion.chunk");
    assert_eq!(chunk.created, 0);
    assert_eq!(chunk.model, MODEL);
    assert_eq!(chunk.choices.len(), 1);
    assert_eq!(chunk.choices[0].index, 0);
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hello"));
    assert_eq!(chunk.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(chunk.choices[0].finish_reason, None);
    assert_eq!(chunk.usage, None);
}

#[tokio::test]
async fn every_chunk_gets_a_fresh_chatcmpl_id() {
    let source = format!("{}{}", content_line("a"), content_line("b"));
    let chunks = chunk_list(&source).await.unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(
        chunks[0].id,
        "chatcmpl-00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(
        chunks[1].id,
        "chatcmpl-00000000-0000-0000-0000-000000000001"
    );
}

#[tokio::test]
async fn a_finished_chunk_carries_the_mapped_reason_and_the_usage() {
    let source = "data: {\"content\": {\"parts\": [{\"text\": \"end\"}]}, \"finish_reason\": \"MAX_TOKENS\", \"usage_metadata\": {\"prompt_token_count\": 2, \"candidates_token_count\": 1, \"total_token_count\": 3}}\n";
    let chunks = chunk_list(source).await.unwrap();
    assert_eq!(chunks.len(), 1);
    let choice = &chunks[0].choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        chunks[0].usage,
        Some(ChatUsage {
            prompt_tokens: 2,
            completion_tokens: 1,
            total_tokens: 3,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        })
    );
}

#[tokio::test]
async fn generic_done_marker() {
    let chunks = chunk_list("data: [DONE]\n\n").await.unwrap();
    assert_eq!(chunks.len(), 1);
    let choice = &chunks[0].choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(choice.delta.content, None);
    assert_eq!(choice.delta.role, None);
    assert_eq!(chunks[0].usage, None);
}

#[tokio::test]
async fn the_done_marker_wins_over_the_rest_of_the_line() {
    let chunks = chunk_list("data: {\"content\": {\"parts\": [{\"text\": \"[DONE]\"}]}}\n")
        .await
        .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(chunks[0].choices[0].delta.content, None);
}

#[rstest]
#[case::empty_object("{}")]
#[case::empty_list("[]")]
#[case::zero("0")]
#[case::falsity("false")]
#[case::null("null")]
#[case::empty_text("\"\"")]
#[tokio::test]
async fn a_falsy_line_is_skipped_without_ending_the_stream(#[case] value: &str) {
    let source = format!("data: {value}\n{}", content_line("after"));
    let chunks = collect(stream_of(&source).await).await.unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("after"));
}

#[rstest]
#[case::event("event: message")]
#[case::not_json("data: not json")]
#[case::no_payload("data:")]
#[case::blank("")]
#[tokio::test]
async fn a_line_that_carries_no_chunk_is_skipped(#[case] line: &str) {
    let source = format!("{line}\n{}", content_line("after"));
    let chunks = collect(stream_of(&source).await).await.unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("after"));
}

#[rstest]
#[case::one("[1]")]
#[case::truth("true")]
#[case::number("1")]
#[case::text("\"x\"")]
fn a_json_value_that_is_not_an_object_ends_the_stream(#[case] value: &str) {
    let source = format!("data: {value}\n{}", content_line("never"));
    let error = runtime().block_on(async {
        expect_error(
            stream_of(&source).await.next_chunk().await,
            "the non-object line was accepted",
        )
    });
    assert!(matches!(error, VertexAgentStreamError::ChunkNotObject));
}

#[tokio::test]
async fn the_line_prefix_is_optional() {
    let chunks = chunk_list("{\"content\": {\"parts\": [{\"text\": \"bare\"}]}}\n")
        .await
        .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("bare"));
}

#[tokio::test]
async fn a_line_split_across_two_reads_is_parsed_as_one() {
    let chunks = collect(
        stream_from(vec![
            Ok(b"data: {\"content\": {\"parts\": [{\"te".to_vec()),
            Ok(b"xt\": \"split\"}]}}\n".to_vec()),
        ])
        .await,
    )
    .await
    .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("split"));
}

#[tokio::test]
async fn a_character_split_across_two_reads_is_decoded() {
    let chunks = collect(
        stream_from(vec![
            Ok(b"data: {\"content\": {\"parts\": [{\"text\": \"h\xc3".to_vec()),
            Ok(b"\xa9llo\"}]}}\n".to_vec()),
        ])
        .await,
    )
    .await
    .unwrap();
    assert_eq!(
        chunks[0].choices[0].delta.content.as_deref(),
        Some("h\u{e9}llo")
    );
}

#[tokio::test]
async fn a_last_line_without_a_newline_is_read() {
    let chunks = chunk_list("data: {\"content\": {\"parts\": [{\"text\": \"tail\"}]}}")
        .await
        .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("tail"));
}

#[tokio::test]
async fn an_empty_body_ends_without_a_chunk() {
    assert!(
        collect(stream_from(Vec::new()).await)
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_chunk_field_of_the_wrong_type_ends_the_stream() {
    let error = runtime().block_on(async {
        expect_error(
            stream_of("data: {\"content\": {\"parts\": [{\"text\": 5}]}}\n")
                .await
                .next_chunk()
                .await,
            "the wrongly typed chunk was accepted",
        )
    });
    assert!(matches!(
        error,
        VertexAgentStreamError::Chunk(AgentEngineStreamError::FieldType { field: "text" })
    ));
}

#[test]
fn a_transport_error_ends_the_stream() {
    let (chunk, error) = runtime().block_on(async {
        let mut stream = stream_from(vec![
            Ok(content_line("one").into_bytes()),
            Err(HttpError::Timeout),
        ])
        .await;
        let chunk = stream.next_chunk().await.unwrap().unwrap();
        let error = expect_error(stream.next_chunk().await, "the read failure was swallowed");
        (chunk, error)
    });
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("one"));
    assert!(matches!(
        error,
        VertexAgentStreamError::Body {
            source: HttpError::Timeout
        }
    ));
}

#[test]
fn an_error_ends_the_stream_even_with_a_partial_line_buffered() {
    let (error, after) = runtime().block_on(async {
        let mut stream = stream_from(vec![
            Ok(b"data: {\"content\": {\"parts\": [{\"text\": \"buffered\"}]}}".to_vec()),
            Err(HttpError::Timeout),
        ])
        .await;
        let error = expect_error(
            stream.next_chunk().await,
            "the buffered line hid the read failure",
        );
        let after = stream.next_chunk().await;
        (error, after)
    });
    assert!(matches!(
        error,
        VertexAgentStreamError::Body {
            source: HttpError::Timeout
        }
    ));
    assert!(after.unwrap().is_none());
}

#[test]
fn an_overlong_line_ends_the_stream() {
    let error = runtime().block_on(async {
        expect_error(
            stream_from(vec![Ok(vec![b'x'; MAX_LINE_BYTES + 1])])
                .await
                .next_chunk()
                .await,
            "the overlong line was accepted",
        )
    });
    assert!(matches!(
        error,
        VertexAgentStreamError::Line(AlephAlphaError::LineTooLong { limit })
            if limit == MAX_LINE_BYTES
    ));
}

const FINISH_LINE: &[u8] = b"data: {\"finish_reason\": \"STOP\"}\n";

async fn after_finish(error: HttpError) -> Result<Option<Chunk>, VertexAgentStreamError> {
    let mut stream = stream_from(vec![Ok(FINISH_LINE.to_vec()), Err(error)]).await;
    let chunk = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some("stop"));
    stream.next_chunk().await
}

#[tokio::test]
async fn a_read_error_after_a_finish_reason_ends_the_stream() {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let source = client.client.get("not a url").build().unwrap_err();
    assert!(!source.is_timeout());
    assert!(
        after_finish(HttpError::Request(source))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        after_finish(HttpError::TooLarge { limit: 1 })
            .await
            .unwrap()
            .is_none()
    );
}

async fn timed_out_request() -> HttpError {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let limits = TransportLimits {
        read_timeout: Duration::from_millis(50),
        ..TransportLimits::DEFAULT
    };
    let client = ProxiedClient::build(base_builder(), &policy, limits).unwrap();
    let source = client
        .client
        .get(format!("http://{address}/"))
        .send()
        .await
        .unwrap_err();
    assert!(source.is_timeout());
    HttpError::Request(source)
}

#[tokio::test]
async fn a_client_timeout_after_a_finish_reason_raises() {
    let after = after_finish(timed_out_request().await).await;
    assert!(matches!(
        expect_error(
            after,
            "a client timeout after the finish reason ended the stream"
        ),
        VertexAgentStreamError::Body { .. }
    ));
}

#[tokio::test]
async fn a_transport_timeout_after_a_finish_reason_raises() {
    let after = after_finish(HttpError::Timeout).await;
    assert!(matches!(
        expect_error(
            after,
            "a transport timeout after the finish reason ended the stream"
        ),
        VertexAgentStreamError::Body { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn a_silent_stream_after_a_finish_reason_times_out() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut stream = opened(held(&dropped, Some(FINISH_LINE.to_vec()))).await;
    let chunk = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some("stop"));
    let error = expect_error(
        stream.next_chunk().await,
        "the silent stream ended cleanly after the finish reason",
    );
    assert!(matches!(error, VertexAgentStreamError::Timeout));
}

#[tokio::test(start_paused = true)]
async fn a_silent_stream_times_out() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut stream = opened(held(&dropped, None)).await;
    let error = expect_error(stream.next_chunk().await, "the silent stream ended cleanly");
    assert!(matches!(error, VertexAgentStreamError::Timeout));
    assert!(stream.next_chunk().await.unwrap().is_none());
}

#[tokio::test]
async fn cancelled_stream_close() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut stream = opened(held(&dropped, Some(content_line("one").into_bytes()))).await;
    let first = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(first.choices[0].delta.content.as_deref(), Some("one"));
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(stream);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_transport_error_surfaces() {
    let data = body();
    let messages = prompt();
    let logger = RecordingLogger::default();
    let default_client: Arc<dyn StreamTransport> = Arc::new(ClosedTransport);
    let deps = StreamDeps {
        supplied: None,
        default_client,
        ids: Arc::new(FixedIds::new()),
        logger: &logger,
    };
    let error = expect_error(
        open(
            &stream_request(&data, &messages, ClientKind::Asynchronous),
            &deps,
        )
        .await,
        "the closed transport answered",
    );
    assert!(matches!(
        error,
        VertexAgentStreamError::Transport {
            source: TransportError::Closed
        }
    ));
}

struct ClosedTransport;

#[async_trait]
impl StreamTransport for ClosedTransport {
    fn kind(&self) -> ClientKind {
        ClientKind::Asynchronous
    }

    async fn post(&self, _request: OutboundRequest) -> Result<StreamHead, TransportError> {
        Err(TransportError::Closed)
    }
}

#[tokio::test]
async fn an_invalid_url_is_refused() {
    let data = body();
    let messages = prompt();
    let logger = RecordingLogger::default();
    let transport = Arc::new(FakeTransport::new(
        ClientKind::Asynchronous,
        200,
        frames(""),
    ));
    let mut request = stream_request(&data, &messages, ClientKind::Asynchronous);
    request.url = "::not a url::";
    let error = expect_error(
        open(&request, &stream_deps(&logger, None, transport)).await,
        "the invalid URL was accepted",
    );
    assert!(matches!(error, VertexAgentStreamError::Url { .. }));
}

async fn serve_once(listener: &TcpListener, reply: &str) -> String {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
    }
    let head_text = String::from_utf8(head).unwrap();
    let length: usize = head_text
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    socket.read_exact(&mut sent).await.unwrap();
    socket.write_all(reply.as_bytes()).await.unwrap();
    head_text
}

#[tokio::test]
async fn the_proxied_client_posts_and_streams_the_reply_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let reply_body = content_line("wire");
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply_body}",
        reply_body.len()
    );
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    assert!(matches!(client.kind(), ClientKind::Asynchronous));
    let request = OutboundRequest {
        method: http::Method::POST,
        url: url::Url::parse(&format!("http://{address}/stream")).unwrap(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(b"{}".to_vec()),
        deadline: Duration::from_secs(5),
    };
    let (head_text, response) = tokio::join!(serve_once(&listener, &reply), client.post(request));
    let head = response.unwrap();
    assert_eq!(head.status, 200);
    let received: Vec<u8> = head.chunks.map(|frame| frame.unwrap()).concat().await;
    assert_eq!(received, reply_body.into_bytes());
    assert!(head_text.starts_with("POST /stream HTTP/1.1"));
}
