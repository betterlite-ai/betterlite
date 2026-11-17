#![cfg(test)]

use std::collections::VecDeque;
use std::slice::from_ref;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_models::decisions;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use indexmap::IndexMap;
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use uuid::Uuid;

use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, UsageIteration, UsageKind,
};
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::messages_stream_split::{AsyncChunkSource, AsyncChunkSplitter, ChunkSource};
use crate::api::messages_stream_usage::state::{StreamSetup, StreamState};
use crate::api::messages_stream_usage::usage::initial_usage;
use crate::api::messages_sync_stream::driver::SyncDriver;

use super::driver::{ChatStreamDriver, MID_STREAM_LOG_PREFIX};
use super::error::ChatStreamError;
use super::sse::{StreamItem, SyncFrames, encode_item};
use super::view::AnthropicSseStream;

const TEXT_START: &str =
    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
const DELTA_A: &str =
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"A"}}"#;
const STOP_0: &str = r#"{"type":"content_block_stop","index":0}"#;
const STOP_1: &str = r#"{"type":"content_block_stop","index":1}"#;
const STOP_2: &str = r#"{"type":"content_block_stop","index":2}"#;
const MESSAGE_STOP: &str = r#"{"type":"message_stop"}"#;
const MESSAGE_DELTA: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":0,"output_tokens":0}}"#;
const HELD_TEXT: &str =
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"H"}}"#;
const HELD_STOP: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":9,"output_tokens":3}}"#;
const THINKING_START: &str = r#"{"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":"","signature":""}}"#;
const THINKING_DELTA: &str =
    r#"{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"R"}}"#;
const TOOL_START: &str = r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call_1","name":"lookup","input":{}}}"#;
const TOOL_DELTA: &str = r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}"#;
const TOOL_START_CALL_2: &str = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_2","name":"lookup","input":{}}}"#;
const STOP_WITH_EDITS: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":5,"output_tokens":2,"iterations":[{"type":"compaction","input_tokens":4,"output_tokens":1},{"type":"message","input_tokens":5,"output_tokens":2}]},"context_management":{"applied_edits":[{"type":"clear_tool_uses_20250919","cleared_tool_uses":1}]}}"#;
const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x1000,
    max_event_bytes: 0x1000,
    max_items: 64,
};

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

#[derive(Clone, Default)]
struct ErrorCount(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for ErrorCount {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() == Level::ERROR {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct Scripted {
    items: VecDeque<Result<StreamChunk, StreamFailure>>,
    pulls: usize,
    chunks: Option<Vec<StreamChunk>>,
    messages: Option<Vec<decisions::Json>>,
}

impl ChunkSource for Scripted {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pulls = self.pulls.saturating_add(1);
        self.items.pop_front().transpose()
    }
}

#[async_trait]
impl AsyncChunkSource for Scripted {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pulls = self.pulls.saturating_add(1);
        self.items.pop_front().transpose()
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.chunks.as_deref()
    }

    fn messages(&self) -> Option<&[decisions::Json]> {
        self.messages.as_deref()
    }
}

type Driver = ChatStreamDriver<Scripted, FixedIds>;
type ByteView = AnthropicSseStream<Scripted, FixedIds>;

type SyncStream = SyncDriver<Scripted, FixedIds>;

fn chunk(source: &str) -> StreamChunk {
    serde_json::from_str(source).expect("a valid chunk fixture")
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("a valid object fixture")
}

fn text(value: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{"content":"{value}"}},"finish_reason":null}}]}}"#
    ))
}

fn finish(reason: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#
    ))
}

fn usage_chunk(prompt: i64, completion: i64) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[],"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion}}}}}"#
    ))
}

fn setup() -> StreamSetup {
    StreamSetup {
        model: "claude-sonnet-4-5".to_owned(),
        tool_name_mapping: IndexMap::new(),
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: None,
    }
}

fn polyfilled() -> StreamSetup {
    let mut filled = setup();
    filled.applied_edits = vec![AppliedEdit {
        edit_type: Some("clear_tool_uses_20250919".to_owned()),
        cleared_input_tokens: None,
        cleared_tool_uses: Some(1),
        cleared_thinking_turns: None,
        summary_input_tokens: None,
        summary_output_tokens: None,
        error: None,
        warnings: None,
    }];
    filled.iterations_usage = Some(vec![
        UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(4),
            output_tokens: Some(1),
        },
        UsageIteration {
            kind: UsageKind::Message,
            input_tokens: Some(5),
            output_tokens: Some(2),
        },
    ]);
    filled
}

fn scripted(chunks: Vec<StreamChunk>) -> Scripted {
    Scripted {
        items: chunks.into_iter().map(Ok).collect(),
        pulls: 0,
        chunks: None,
        messages: None,
    }
}

fn failing(chunks: Vec<StreamChunk>, failure: StreamFailure) -> Scripted {
    let mut items: VecDeque<Result<StreamChunk, StreamFailure>> =
        chunks.into_iter().map(Ok).collect();
    items.push_back(Err(failure));
    Scripted {
        items,
        pulls: 0,
        chunks: None,
        messages: None,
    }
}

fn driver(source: Scripted, setup: StreamSetup) -> Driver {
    ChatStreamDriver {
        state: StreamState::new(
            None,
            AsyncChunkSplitter {
                source,
                buffer: VecDeque::new(),
            },
            setup,
        ),
        ids: FixedIds,
    }
}

fn started(chunks: Vec<StreamChunk>) -> Driver {
    driver(scripted(chunks), setup())
}

fn byte_view(source: Scripted, setup: StreamSetup) -> ByteView {
    AnthropicSseStream::new(driver(source, setup), LIMITS)
}

fn sync_frames(chunks: Vec<StreamChunk>, message_id: &str) -> SyncStream {
    let source = scripted(chunks);
    let mut driver = SyncDriver {
        state: StreamState::new(None, source, setup()),
        ids: FixedIds,
    };
    message_id.clone_into(&mut driver.state.message_id);
    driver
}

async fn next(stream: &mut Driver) -> Object {
    stream
        .next_event()
        .await
        .expect("the stream reports no failure")
        .expect("the stream still has events")
}

async fn take(stream: &mut Driver, count: usize) -> Vec<Object> {
    let mut events = Vec::with_capacity(count);
    for _ in 0..count {
        events.push(next(stream).await);
    }
    events
}

async fn assert_events(stream: &mut Driver, frames: &[&str]) {
    let expected: Vec<Object> = frames.iter().copied().map(object).collect();
    assert_eq!(take(stream, expected.len()).await, expected);
}

fn pulls(stream: &Driver) -> usize {
    stream.state.completion_stream.source.source.pulls
}

async fn assert_message(stream: &mut Driver) {
    let start = next(stream).await;
    let announced = start
        .get("message")
        .and_then(Json::as_object)
        .expect("message_start carries the message");
    assert_eq!(
        start.get("type").and_then(Json::as_str),
        Some("message_start")
    );
    assert_eq!(
        announced.get("model").and_then(Json::as_str),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(
        announced.get("role").and_then(Json::as_str),
        Some("assistant")
    );
    assert_eq!(announced.get("content"), Some(&Json::List(Vec::new())));
    assert_eq!(announced.get("stop_reason"), Some(&Json::Null));
    assert_eq!(announced.get("stop_sequence"), Some(&Json::Null));
    assert_eq!(announced.get("usage"), Some(&Json::Object(initial_usage())));
    let id = announced
        .get("id")
        .and_then(Json::as_str)
        .expect("the message names its id");
    let suffix = id.strip_prefix("msg_").expect("the msg_ prefix");
    Uuid::parse_str(suffix).expect("the message id holds a uuid");
    assert_eq!(id, stream.state.message_id.as_str());
}

async fn drain_frames(view: &mut ByteView, limit: usize) -> Vec<String> {
    let mut frames = Vec::new();
    while frames.len() < limit {
        let Some(frame) = view.next_frame().await else {
            break;
        };
        frames.push(String::from_utf8(frame).expect("the frame is text"));
    }
    frames
}

#[tokio::test]
async fn async_event_lifecycle() {
    let reasoning = chunk(
        r#"{"choices":[{"index":0,"delta":{"reasoning_content":"R"},"finish_reason":null}]}"#,
    );
    let tool = chunk(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"x\":1}"}}]},"finish_reason":null}]}"#,
    );
    let mut stream = started(vec![text("A"), reasoning, tool, finish("stop")]);
    assert_message(&mut stream).await;
    assert_eq!(pulls(&stream), 0, "the first event needs no pull");
    assert_events(&mut stream, &[TEXT_START, DELTA_A]).await;
    assert_events(
        &mut stream,
        &[
            STOP_0,
            THINKING_START,
            THINKING_DELTA,
            STOP_1,
            TOOL_START,
            TOOL_DELTA,
            STOP_2,
            MESSAGE_DELTA,
            MESSAGE_STOP,
        ],
    )
    .await;
    assert!(stream.next_event().await.expect("no failure").is_none());
}

#[tokio::test]
async fn async_named_tool_with_empty_arguments_starts_at_once() {
    let empty = chunk(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_2","type":"function","function":{"name":"lookup","arguments":""}}]},"finish_reason":null}]}"#,
    );
    let mut stream = started(vec![empty]);
    assert_message(&mut stream).await;
    assert_events(&mut stream, &[TOOL_START_CALL_2]).await;
    assert_eq!(pulls(&stream), 1, "the empty delta flushed the start");
    assert_events(&mut stream, &[MESSAGE_STOP]).await;
}

#[tokio::test]
async fn async_blank_chunks_open_no_block() {
    let blank = chunk(r#"{"choices":[{"index":0,"delta":{},"finish_reason":null}]}"#);
    let mut stream = started(vec![blank, usage_chunk(5, 2), text("A")]);
    assert_message(&mut stream).await;
    assert_events(&mut stream, &[TEXT_START, DELTA_A]).await;
    assert_eq!(
        pulls(&stream),
        3,
        "blank and choiceless metadata open no block"
    );
}

#[tokio::test]
async fn async_compaction_block_precedes_ordinary_content() {
    let mut stream = driver(
        scripted(vec![text("A")]),
        StreamSetup {
            compaction_block: Some(CompactionBlock {
                kind: CompactionKind::Compaction,
                content: Some("S".to_owned()),
                signature: None,
            }),
            ..setup()
        },
    );
    assert_message(&mut stream).await;
    assert_events(
        &mut stream,
        &[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"compaction","content":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":"S"}}"#,
            STOP_0,
        ],
    )
    .await;
    assert_eq!(pulls(&stream), 0, "the synthetic block needs no read");
    assert_events(
        &mut stream,
        &[r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#],
    )
    .await;
    assert_eq!(pulls(&stream), 1);
}

#[tokio::test]
async fn async_terminal_usage() {
    let mut stream = driver(
        scripted(vec![
            text("A"),
            finish("stop"),
            usage_chunk(5, 2),
            text("B"),
        ]),
        polyfilled(),
    );
    assert_message(&mut stream).await;
    assert_events(
        &mut stream,
        &[TEXT_START, DELTA_A, STOP_0, STOP_WITH_EDITS, MESSAGE_STOP],
    )
    .await;
    assert!(
        stream.next_event().await.expect("no failure").is_none(),
        "the trailing content is dropped after the usage delta"
    );
    assert_eq!(pulls(&stream), 5, "the trailing chunk was still read");
}

#[tokio::test]
async fn async_finish_without_usage_flushes() {
    let mut stream = started(vec![text("A"), finish("stop")]);
    assert_message(&mut stream).await;
    assert_events(
        &mut stream,
        &[TEXT_START, DELTA_A, STOP_0, MESSAGE_DELTA, MESSAGE_STOP],
    )
    .await;
    assert!(
        stream.next_event().await.expect("no failure").is_none(),
        "message_stop leaves once"
    );
}

#[tokio::test]
async fn async_held_content_comes_before_the_flushed_delta() {
    let mut stream = started(Vec::new());
    stream.state.message_progress.sent_first_chunk = true;
    stream.state.holding_chunk = Some(object(HELD_TEXT));
    stream.state.holding_stop_reason_chunk = Some(object(HELD_STOP));
    assert_events(&mut stream, &[HELD_TEXT, STOP_0, HELD_STOP, MESSAGE_STOP]).await;
    assert!(stream.state.holding_stop_reason_chunk.is_none());
    assert!(
        stream.next_event().await.expect("no failure").is_none(),
        "the held delta is not re-emitted"
    );
}

#[tokio::test]
async fn sse_frame_encoding() {
    let mut view = byte_view(scripted(Vec::new()), setup());
    view.stream
        .state
        .chunk_queue
        .push_back(object(r#"{"type":"ping"}"#));
    assert_eq!(
        view.next_frame().await.expect("the queued ping"),
        b"event: ping\ndata: {\"type\": \"ping\"}\n\n".to_vec()
    );
    view.stream
        .state
        .chunk_queue
        .push_back(object(r#"{"a":1}"#));
    assert_eq!(
        view.next_frame().await.expect("the untyped chunk"),
        b"event: message\ndata: {\"a\": 1}\n\n".to_vec()
    );
    view.stream
        .state
        .chunk_queue
        .push_back(object(r#"{"type":null}"#));
    assert_eq!(
        view.next_frame().await.expect("the null type"),
        b"event: None\ndata: {\"type\": null}\n\n".to_vec()
    );
}

#[tokio::test]
async fn sse_frame_escapes_non_ascii() {
    let mut view = byte_view(scripted(Vec::new()), setup());
    view.stream
        .state
        .chunk_queue
        .push_back(object(r#"{"type":"ping","text":"\u00e9"}"#));
    let escaped = view.next_frame().await.expect("the non-ascii text");
    assert_eq!(
        escaped,
        b"event: ping\ndata: {\"type\": \"ping\", \"text\": \"\\u00e9\"}\n\n".to_vec()
    );
    let escaped_text = String::from_utf8(escaped).expect("the frame is text");
    let payload = escaped_text
        .lines()
        .nth(1)
        .and_then(|line| line.strip_prefix("data: "))
        .expect("the data line");
    let decoded: Object = serde_json::from_str(payload).expect("the payload is an object");
    assert_eq!(decoded.get("text"), Some(&Json::Text("\u{e9}".to_owned())));
}

#[tokio::test]
async fn sse_refused_type_ends_the_view() {
    let mut refused = byte_view(scripted(Vec::new()), setup());
    refused
        .stream
        .state
        .chunk_queue
        .push_back(object(r#"{"type":5}"#));
    assert_eq!(
        refused.next_frame().await.expect("the refused type"),
        b"event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"api_error\", \"message\": \"the event cannot name an SSE event: the chunk's type must be text or null\"}}\n\n"
            .to_vec()
    );
    assert!(
        refused.next_frame().await.is_none(),
        "the refused frame ends the view"
    );
}

#[tokio::test]
async fn sse_bytes_pass_through() {
    let raw = b"event: raw\ndata: {}\n\n".to_vec();
    assert_eq!(
        encode_item(StreamItem::Bytes(raw.clone())).expect("bytes pass through"),
        raw
    );
}

#[tokio::test]
async fn sync_frames_stop_after_the_delegated_view() {
    let mut sync = SyncFrames {
        driver: sync_frames(vec![text("A")], "msg_fixed"),
    };
    let encoded: Vec<Vec<u8>> = sync
        .by_ref()
        .take(8)
        .collect::<Result<_, _>>()
        .expect("every frame encodes");
    assert_eq!(
        encoded.len(),
        4,
        "message_start, the block start, the delta, then message_stop"
    );
    assert!(
        sync.next().is_none(),
        "the synchronous wrapper ends its iteration"
    );
    let mut delegated_view = byte_view(scripted(vec![text("A")]), setup());
    "msg_fixed".clone_into(&mut delegated_view.stream.state.message_id);
    let mut delegated = Vec::new();
    while delegated.len() < 8 {
        let Some(frame) = delegated_view.next_frame().await else {
            break;
        };
        delegated.push(frame);
    }
    assert_eq!(encoded, delegated);
    assert!(
        delegated_view.next_frame().await.is_none(),
        "the delegated view ends where the wrapper does"
    );
}

#[tokio::test]
async fn async_failure_reaches_the_direct_iterator() {
    let errors = ErrorCount::default();
    let _subscription = set_default(tracing_subscriber::registry().with(errors.clone()));
    let reported = StreamFailure::Reported {
        status_code: 429,
        message: "busy".to_owned(),
    };
    let mut stream = driver(failing(vec![text("A")], reported), setup());
    assert_message(&mut stream).await;
    assert_events(&mut stream, &[TEXT_START, DELTA_A]).await;
    let failure = stream
        .next_event()
        .await
        .expect_err("the direct iteration hands the failure over");
    match failure {
        ChatStreamError::Stream {
            failure:
                StreamFailure::Reported {
                    status_code,
                    message,
                },
        } => {
            assert_eq!(status_code, 429);
            assert_eq!(message, "busy");
        }
        other @ (ChatStreamError::Stream { .. } | ChatStreamError::State(_)) => {
            panic!("expected the reported failure, got {other:?}");
        }
    }
    assert_eq!(
        errors.0.load(Ordering::Relaxed),
        0,
        "the driver logs nothing"
    );
}

#[tokio::test]
async fn async_error_event_ends_stream() {
    assert_eq!(
        MID_STREAM_LOG_PREFIX,
        "Anthropic Adapter - mid-stream error, emitting Anthropic error event: "
    );
    let errors = ErrorCount::default();
    let _subscription = set_default(tracing_subscriber::registry().with(errors.clone()));
    let reported = StreamFailure::Reported {
        status_code: 429,
        message: "busy".to_owned(),
    };
    let mut view = byte_view(failing(vec![text("A")], reported), setup());
    let frames = drain_frames(&mut view, 8).await;
    assert_eq!(
        frames.len(),
        4,
        "message_start, the block start, the delta, then the error"
    );
    assert!(
        frames
            .iter()
            .all(|frame| !frame.contains("message_stop") && !frame.contains("message_delta")),
        "no terminal frame is fabricated after the failure"
    );
    assert_eq!(
        frames.last().map(String::as_str),
        Some(
            "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"rate_limit_error\", \"message\": \"busy\"}}\n\n"
        )
    );
    assert!(
        view.next_frame().await.is_none(),
        "the error frame ended the stream"
    );
    assert_eq!(errors.0.load(Ordering::Relaxed), 1, "one error line");
}

#[tokio::test]
async fn async_empty_failure_event_ends_stream() {
    let errors = ErrorCount::default();
    let _subscription = set_default(tracing_subscriber::registry().with(errors.clone()));
    let empty = StreamFailure::Unclassified {
        text: String::new(),
    };
    let mut view = byte_view(failing(vec![text("A")], empty), setup());
    let frames = drain_frames(&mut view, 8).await;
    assert_eq!(
        frames.last().map(String::as_str),
        Some(
            "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"api_error\", \"message\": \"Upstream stream ended before completion\"}}\n\n"
        )
    );
    assert_eq!(errors.0.load(Ordering::Relaxed), 1, "one line per boundary");
}

#[tokio::test]
async fn async_state_error_reaches_the_caller() {
    let malformed =
        chunk(r#"{"choices":[{"index":0,"delta":{"thinking_blocks":[5]},"finish_reason":null}]}"#);
    let mut stream = driver(scripted(vec![malformed]), setup());
    assert_message(&mut stream).await;
    let state_failure = stream
        .next_event()
        .await
        .expect_err("the state error reaches the caller");
    match state_failure {
        ChatStreamError::State(error) => assert!(
            error
                .to_string()
                .starts_with("the event is not valid JSON: "),
            "the state error keeps its own text"
        ),
        other @ ChatStreamError::Stream { .. } => {
            panic!("expected the state error, got {other:?}");
        }
    }
}

#[tokio::test]
async fn async_state_error_event_ends_stream() {
    let errors = ErrorCount::default();
    let _subscription = set_default(tracing_subscriber::registry().with(errors.clone()));
    let malformed =
        chunk(r#"{"choices":[{"index":0,"delta":{"thinking_blocks":[5]},"finish_reason":null}]}"#);
    let mut view = byte_view(scripted(vec![malformed]), setup());
    let start = String::from_utf8(view.next_frame().await.expect("the message_start frame"))
        .expect("the frame is text");
    assert!(start.starts_with("event: message_start\n"));
    let state_frame = String::from_utf8(view.next_frame().await.expect("the state error event"))
        .expect("the frame is text");
    assert!(
        state_frame.starts_with(
            "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"api_error\", \"message\": \"the event is not valid JSON: "
        ),
        "the state error becomes the one error event"
    );
    assert!(
        view.next_frame().await.is_none(),
        "the state error ended the stream"
    );
    assert_eq!(errors.0.load(Ordering::Relaxed), 1, "one line per boundary");
}

#[tokio::test]
async fn async_oversized_error_frame_is_dropped() {
    let errors = ErrorCount::default();
    let _subscription = set_default(tracing_subscriber::registry().with(errors.clone()));
    let oversized = StreamFailure::Reported {
        status_code: 500,
        message: "x".repeat(0x2000),
    };
    let mut view = byte_view(failing(Vec::new(), oversized), setup());
    let start = String::from_utf8(view.next_frame().await.expect("the message_start frame"))
        .expect("the frame is text");
    assert!(start.starts_with("event: message_start\n"));
    assert!(
        view.next_frame().await.is_none(),
        "an error event past the wire limits is not written"
    );
    assert!(
        view.next_frame().await.is_none(),
        "the stream stays ended after the dropped frame"
    );
    assert_eq!(
        errors.0.load(Ordering::Relaxed),
        2,
        "the failure line and the oversized-frame line"
    );
}

#[tokio::test]
async fn async_byte_view_metadata() {
    let messages = vec![decisions::Json::Object(IndexMap::from([
        ("role".to_owned(), decisions::Json::Text("user".to_owned())),
        ("content".to_owned(), decisions::Json::Text("hi".to_owned())),
    ]))];
    let recorded = text("A");
    let mut source = scripted(vec![text("A")]);
    source.chunks = Some(vec![recorded.clone()]);
    source.messages = Some(messages.clone());
    let mut view = byte_view(source, setup());
    assert!(
        view.hidden_params.is_empty(),
        "the constructor starts the view with none"
    );
    view.hidden_params.insert(
        "cache_key".to_owned(),
        decisions::Json::Text("k".to_owned()),
    );
    assert_eq!(view.model(), "claude-sonnet-4-5");
    assert_eq!(
        view.chunks(),
        Some(from_ref(&recorded)),
        "the view hands the recorded chunks back unchanged"
    );
    assert_eq!(view.messages(), Some(messages.as_slice()));
    let frame = view.next_frame().await.expect("the message_start frame");
    let encoded = String::from_utf8(frame).expect("the frame is text");
    assert!(encoded.starts_with("event: message_start\ndata: {"));
    assert!(encoded.contains("\"model\": \"claude-sonnet-4-5\""));
}

#[tokio::test]
async fn async_aclose_closes_the_generator_only() {
    let mut view = byte_view(scripted(vec![text("A")]), setup());
    view.hidden_params.insert(
        "cache_key".to_owned(),
        decisions::Json::Text("k".to_owned()),
    );
    view.aclose();
    assert!(
        view.next_frame().await.is_none(),
        "a closed view yields nothing"
    );
    view.aclose();
    assert!(
        view.next_frame().await.is_none(),
        "closing twice changes nothing"
    );
    assert_eq!(
        view.hidden_params.get("cache_key"),
        Some(&decisions::Json::Text("k".to_owned())),
        "the hidden params the proxy stamped survive aclose"
    );
    assert!(
        view.stream
            .next_event()
            .await
            .expect("the provider stream stays open")
            .is_some(),
        "aclose does not close the completion stream"
    );
}

#[tokio::test]
async fn async_views_keep_their_own_setup() {
    let mut first_setup = setup();
    first_setup.model = "model-a".to_owned();
    first_setup
        .tool_name_mapping
        .insert("lookup".to_owned(), "lookup_a".to_owned());
    let mut second_setup = setup();
    second_setup.model = "model-b".to_owned();
    let first_tool_chunk = chunk(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"lookup","arguments":"{\"x\":1}"}}]},"finish_reason":null}]}"#,
    );
    let second_tool_chunk = chunk(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_b","type":"function","function":{"name":"lookup","arguments":"{\"x\":1}"}}]},"finish_reason":null}]}"#,
    );
    let mut first = byte_view(scripted(vec![first_tool_chunk]), first_setup);
    let mut second = byte_view(scripted(vec![second_tool_chunk]), second_setup);
    let first_start = String::from_utf8(first.next_frame().await.expect("its own start"))
        .expect("the frame is text");
    let second_start = String::from_utf8(second.next_frame().await.expect("its own start"))
        .expect("the frame is text");
    assert!(first_start.contains("\"model\": \"model-a\""));
    assert!(second_start.contains("\"model\": \"model-b\""));
    let first_tool = String::from_utf8(first.next_frame().await.expect("first tool start"))
        .expect("the frame is text");
    let second_tool = String::from_utf8(second.next_frame().await.expect("second tool start"))
        .expect("the frame is text");
    assert!(first_tool.contains("\"name\": \"lookup_a\""));
    assert!(second_tool.contains("\"name\": \"lookup\""));
    assert_ne!(
        first.stream.state.message_id,
        second.stream.state.message_id
    );
}

#[tokio::test]
async fn async_queued_events_stay_in_their_view() {
    let mut first = byte_view(scripted(vec![text("A")]), setup());
    let mut second = byte_view(scripted(vec![text("A")]), setup());
    first
        .stream
        .state
        .chunk_queue
        .push_back(object(r#"{"type":"ping"}"#));
    let ping = String::from_utf8(first.next_frame().await.expect("the queued ping"))
        .expect("the frame is text");
    assert!(ping.starts_with("event: ping\n"));
    let first_start = String::from_utf8(first.next_frame().await.expect("its own start"))
        .expect("the frame is text");
    let second_start = String::from_utf8(second.next_frame().await.expect("its own start"))
        .expect("the frame is text");
    assert!(first_start.starts_with("event: message_start\n"));
    assert!(
        !second_start.starts_with("event: ping"),
        "the ping stayed in its view"
    );
}
