#![cfg(test)]

use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use uuid::Uuid;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_stream_errors::failure::FailureCause;

use super::source::ResponsesEventSource;
use super::{MessagesResponseStream, MessagesResponseStreamInput, MessagesResponseStreamResult};

const MODEL: &str = "model_a";
const MESSAGE_ID: &str = "msg_00000000-0000-0000-0000-000000000007";
const MESSAGE_ITEM: &str =
    r#"{"type":"response.output_item.added","item":{"type":"message","id":"item_a"}}"#;
const TEXT_DELTA: &str = r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"hi"}"#;
const NON_ASCII_DELTA: &str =
    r#"{"type":"response.output_text.delta","item_id":"item_a","delta":"é"}"#;
const TOOL_ITEM: &str = r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"item_tool","call_id":"call_a","name":"lookup"}}"#;
const ARGS_DELTA: &str =
    r#"{"type":"response.function_call_arguments.delta","item_id":"item_tool","delta":"1}"}"#;
const TOOL_DONE: &str =
    r#"{"type":"response.output_item.done","item":{"type":"function_call","id":"item_tool"}}"#;
const CREATED: &str = r#"{"type":"response.created"}"#;
const COMPLETED: &str = r#"{"type":"response.completed"}"#;
const TEXT_BLOCK_START: &str =
    r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
const TEXT_BLOCK_DELTA: &str =
    r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#;
const TOOL_BLOCK_START: &str = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_a","name":"lookup","input":{}}}"#;
const ARGS_BLOCK_DELTA: &str = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"1}"}}"#;
const TOOL_BLOCK_STOP: &str = r#"{"type":"content_block_stop","index":1}"#;
const MESSAGE_DELTA: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":0,"output_tokens":0}}"#;
const MESSAGE_STOP: &str = r#"{"type":"message_stop"}"#;
const INCOMPLETE_ERROR: &str = r#"{"type":"error","error":{"type":"api_error","message":"Provider stream ended before emitting a message_stop event; the response is incomplete and any partial content (e.g. tool_use input JSON) may be truncated."}}"#;
const EOF_LOG: &str = "AnthropicResponsesStreamWrapper: upstream Responses stream for model_a ended without a terminal event";
const PULL_FAILED_LOG: &str =
    "AnthropicResponsesStreamWrapper: upstream Responses stream for model_a failed";

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

#[derive(Clone)]
struct Scripted {
    events: VecDeque<Json>,
    failure: Option<FailureCause>,
    pulls: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

impl Drop for Scripted {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl ResponsesEventSource for Scripted {
    async fn next_event(&mut self) -> Result<Option<Json>, FailureCause> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        match self.failure.take() {
            Some(failure) => Err(failure),
            None => Ok(None),
        }
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Logged(String);

impl Visit for Logged {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let mut logged = Logged::default();
        event.record(&mut logged);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(logged.0);
    }
}

fn capture() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    (set_default(subscriber), captured)
}

fn lines(captured: &Captured) -> Vec<String> {
    captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a valid fixture")
}

fn cause(source: &str) -> FailureCause {
    serde_json::from_str(source).expect("a valid failure fixture")
}

fn scripted(events: &[&str]) -> Scripted {
    Scripted {
        events: events.iter().map(|source| json(source)).collect(),
        failure: None,
        pulls: Arc::default(),
        closed: Arc::default(),
    }
}

fn stream_with(
    source: Scripted,
    model: &str,
    logging: Option<&mut LoggingState>,
) -> MessagesResponseStream<Scripted> {
    MessagesResponseStream::new(
        source,
        &FixedIds,
        logging,
        MessagesResponseStreamInput {
            model: model.to_owned(),
            redactor: None,
            limits: ApiLimits::default(),
        },
    )
}

fn stream(source: Scripted) -> MessagesResponseStream<Scripted> {
    stream_with(source, MODEL, None)
}

fn redacting_stream(source: Scripted) -> MessagesResponseStream<Scripted> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in patterns compile");
    MessagesResponseStream::new(
        source,
        &FixedIds,
        None,
        MessagesResponseStreamInput {
            model: MODEL.to_owned(),
            redactor: Some(Arc::new(redactor)),
            limits: ApiLimits::default(),
        },
    )
}

fn message_start() -> Json {
    json(concat!(
        r#"{"type":"message_start","message":{"id":"msg_00000000-0000-0000-0000-000000000007","#,
        r#""type":"message","role":"assistant","content":[],"model":"model_a","stop_reason":null,"#,
        r#""stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0,"#,
        r#""cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#,
    ))
}

async fn next(stream: &mut MessagesResponseStream<Scripted>) -> Json {
    match stream
        .next_chunk()
        .await
        .expect("the wrapper yields a chunk")
    {
        MessagesResponseStreamResult::Chunk(chunk) => chunk,
        MessagesResponseStreamResult::End => panic!("the stream ended before its last chunk"),
    }
}

async fn ended(stream: &mut MessagesResponseStream<Scripted>) {
    let result = stream.next_chunk().await.expect("the wrapper ends cleanly");
    assert_eq!(result, MessagesResponseStreamResult::End);
}

async fn drain(stream: &mut MessagesResponseStream<Scripted>) -> Vec<Json> {
    let mut chunks = Vec::new();
    loop {
        match stream
            .next_chunk()
            .await
            .expect("the wrapper yields a chunk")
        {
            MessagesResponseStreamResult::Chunk(chunk) => chunks.push(chunk),
            MessagesResponseStreamResult::End => return chunks,
        }
    }
}

async fn sse_frames(stream: &mut MessagesResponseStream<Scripted>) -> Vec<String> {
    let mut frames = Vec::new();
    loop {
        let Some(frame) = stream
            .next_frame()
            .await
            .expect("the wrapper frames a chunk")
        else {
            return frames;
        };
        frames.push(String::from_utf8(frame).expect("a frame is UTF-8"));
    }
}

// The wrapper returns its pre-sent message_start before it reads any upstream event.
#[tokio::test]
async fn message_start_comes_before_the_first_pull() {
    let mut source = scripted(&[TEXT_DELTA]);
    let pulls = Arc::clone(&source.pulls);
    let mut stream = stream(source);
    let first = next(&mut stream).await;
    assert_eq!(first, message_start());
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        0,
        "the first chunk reads no event"
    );
}

// __init__ records the msg_ id it minted on the caller's logging state before any event arrives.
#[tokio::test]
async fn the_message_id_is_recorded_on_the_logging_state() {
    let mut logging = logging_state();
    let stream = stream_with(scripted(&[]), MODEL, Some(&mut logging));
    assert_eq!(
        logging.streamed_anthropic_message_id.as_deref(),
        Some(MESSAGE_ID)
    );
    drop(stream);
}

// Two chunks queued by one event: the second is returned from the queue without another pull.
#[tokio::test]
async fn queued_chunks_are_returned_before_the_next_pull() {
    let mut source = scripted(&[COMPLETED]);
    let pulls = Arc::clone(&source.pulls);
    let mut stream = stream(source);
    assert_eq!(next(&mut stream).await, message_start());
    assert_eq!(next(&mut stream).await, json(MESSAGE_DELTA));
    assert_eq!(next(&mut stream).await, json(MESSAGE_STOP));
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        1,
        "the terminal pair came from one event"
    );
    ended(&mut stream).await;
}

// A failed event queues its error chunk, and the next call ends the iteration without a pull.
#[tokio::test]
async fn a_failed_event_ends_the_iteration() {
    let mut source = scripted(&[
        r#"{"type":"response.failed","response":{"error":{"type":"rate_limit_error","message":"busy"}}}"#,
    ]);
    let pulls = Arc::clone(&source.pulls);
    let mut stream = stream(source);
    assert_eq!(next(&mut stream).await, message_start());
    assert_eq!(
        next(&mut stream).await,
        json(r#"{"type":"error","error":{"type":"rate_limit_error","message":"busy"}}"#)
    );
    assert!(stream.state.snapshot().stream_failed.0);
    ended(&mut stream).await;
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        1,
        "a failed stream is not pulled again"
    );
}

// terminal failed to error: a failed event becomes the terminal error chunk with its own status and message.
#[rstest]
#[case::reported_status(
    r#"{"type":"response.failed","response":{"error":{"type":"rate_limit_error","message":"busy"}}}"#,
    r#"{"type":"error","error":{"type":"rate_limit_error","message":"busy"}}"#
)]
#[case::missing_response(
    r#"{"type":"response.failed"}"#,
    r#"{"type":"error","error":{"type":"api_error","message":"Response API in-stream error"}}"#
)]
#[tokio::test]
async fn terminal_failed_to_error(#[case] event: &str, #[case] expected: &str) {
    let mut stream = stream(scripted(&[event]));
    assert_eq!(next(&mut stream).await, message_start());
    assert_eq!(next(&mut stream).await, json(expected));
    let snapshot = stream.state.snapshot();
    assert!(snapshot.stream_failed.0, "the stream is marked failed");
    assert!(
        !snapshot.sent_message_stop.0,
        "a failure never sends message_stop"
    );
    ended(&mut stream).await;
}

// early eof error: an upstream that ends without message_stop becomes the 500 incomplete-stream error and is logged.
#[rstest]
#[case::reads_no_event(vec![], vec![INCOMPLETE_ERROR])]
#[case::ends_after_a_delta(
    vec![MESSAGE_ITEM, TEXT_DELTA],
    vec![TEXT_BLOCK_START, TEXT_BLOCK_DELTA, INCOMPLETE_ERROR]
)]
#[tokio::test]
async fn early_eof_error(#[case] events: Vec<&str>, #[case] expected: Vec<&str>) {
    let (guard, captured) = capture();
    let mut stream = stream(scripted(&events));
    assert_eq!(next(&mut stream).await, message_start());
    let chunks = drain(&mut stream).await;
    let wanted: Vec<Json> = expected.iter().map(|source| json(source)).collect();
    assert_eq!(chunks, wanted);
    assert!(stream.state.snapshot().stream_failed.0);
    drop(guard);
    assert_eq!(lines(&captured), vec![EOF_LOG.to_owned()]);
}

// An upstream exception is logged and becomes a failure chunk with its reported status and message.
#[rstest]
#[case::reported_status(
    r#"{"kind":"plain","text":"upstream exploded","message":"overloaded","status_code":529}"#,
    r#"{"type":"error","error":{"type":"overloaded_error","message":"overloaded"}}"#
)]
#[case::text_without_status(
    r#"{"kind":"plain","text":"upstream exploded"}"#,
    r#"{"type":"error","error":{"type":"api_error","message":"upstream exploded"}}"#
)]
#[tokio::test]
async fn upstream_failure_to_error(#[case] failure: &str, #[case] expected: &str) {
    let (guard, captured) = capture();
    let mut source = scripted(&[CREATED]);
    source.failure = Some(cause(failure));
    let mut stream = stream(source);
    assert_eq!(next(&mut stream).await, message_start());
    assert_eq!(next(&mut stream).await, json(expected));
    assert!(stream.state.snapshot().stream_failed.0);
    ended(&mut stream).await;
    drop(guard);
    assert_eq!(lines(&captured), vec![PULL_FAILED_LOG.to_owned()]);
}

// An event B1119 rejects is an exception inside the pull, so its text becomes the failure message.
#[tokio::test]
async fn a_rejected_event_becomes_the_error_chunk() {
    let (guard, captured) = capture();
    let mut stream = stream(scripted(&[r#"{"type":"response.failed","response":5}"#]));
    assert_eq!(next(&mut stream).await, message_start());
    assert_eq!(
        next(&mut stream).await,
        json(
            r#"{"type":"error","error":{"type":"api_error","message":"a response.failed event carries a response that is not a mapping or an object"}}"#
        )
    );
    assert!(stream.state.snapshot().stream_failed.0);
    ended(&mut stream).await;
    drop(guard);
    assert_eq!(lines(&captured), vec![PULL_FAILED_LOG.to_owned()]);
}

// A stream that reached message_stop ends quietly, with no error chunk and no log line.
#[tokio::test]
async fn a_completed_stream_ends_without_an_error() {
    let (guard, captured) = capture();
    let mut stream = stream(scripted(&[COMPLETED]));
    assert_eq!(next(&mut stream).await, message_start());
    assert_eq!(
        drain(&mut stream).await,
        vec![json(MESSAGE_DELTA), json(MESSAGE_STOP)]
    );
    drop(guard);
    assert!(
        lines(&captured).is_empty(),
        "a terminal stream logs nothing"
    );
}

// LiteLLM does not suppress a second completed event, so the wrapper re-emits its terminal pair.
#[tokio::test]
async fn duplicate_completed_events_are_reemitted() {
    let mut stream = stream(scripted(&[COMPLETED, COMPLETED]));
    assert_eq!(next(&mut stream).await, message_start());
    let chunks = drain(&mut stream).await;
    let pair = [json(MESSAGE_DELTA), json(MESSAGE_STOP)];
    let mut doubled = pair.to_vec();
    doubled.extend(pair);
    assert_eq!(chunks, doubled);
}

// Text, tool arguments, the block stop and the terminal pair keep their order and block indexes.
#[tokio::test]
async fn block_indexes_survive_the_queue() {
    let events = [
        MESSAGE_ITEM,
        TEXT_DELTA,
        TOOL_ITEM,
        ARGS_DELTA,
        TOOL_DONE,
        COMPLETED,
    ];
    let mut stream = stream(scripted(&events));
    assert_eq!(next(&mut stream).await, message_start());
    let chunks = drain(&mut stream).await;
    assert_eq!(
        chunks,
        vec![
            json(TEXT_BLOCK_START),
            json(TEXT_BLOCK_DELTA),
            json(TOOL_BLOCK_START),
            json(ARGS_BLOCK_DELTA),
            json(TOOL_BLOCK_STOP),
            json(MESSAGE_DELTA),
            json(MESSAGE_STOP),
        ]
    );
}

// async_anthropic_sse_wrapper writes the event line and the data line Python's json.dumps produces.
#[tokio::test]
async fn frames_carry_the_event_and_data_lines() {
    let mut stream = stream(scripted(&[COMPLETED]));
    let frames = sse_frames(&mut stream).await;
    assert_eq!(
        frames,
        vec![
            MESSAGE_START_FRAME.to_owned(),
            MESSAGE_DELTA_FRAME.to_owned(),
            MESSAGE_STOP_FRAME.to_owned(),
        ]
    );
}

// A non-ASCII delta is escaped in the data line, as Python's json.dumps escapes it.
#[tokio::test]
async fn frames_escape_non_ascii() {
    let mut stream = stream(scripted(&[MESSAGE_ITEM, NON_ASCII_DELTA]));
    assert_eq!(next(&mut stream).await, message_start());
    let frames = sse_frames(&mut stream).await;
    assert_eq!(frames[1], NON_ASCII_FRAME);
}

// Dropping the wrapper releases the upstream it was given, which cancels the provider stream.
#[tokio::test]
async fn dropping_the_stream_drops_the_upstream() {
    let mut source = scripted(&[]);
    let closed = Arc::clone(&source.closed);
    let mut stream = stream(source);
    assert_eq!(next(&mut stream).await, message_start());
    assert!(!closed.load(Ordering::SeqCst));
    drop(stream);
    assert!(
        closed.load(Ordering::SeqCst),
        "the upstream is released with the wrapper"
    );
}

// Each wrapper owns its queue and flags, so draining one leaves the other where it was.
#[tokio::test]
async fn the_wrappers_do_not_share_a_queue() {
    let mut first = stream(scripted(&[COMPLETED]));
    let mut second = stream(scripted(&[]));
    assert_eq!(next(&mut first).await, message_start());
    assert_eq!(next(&mut second).await, message_start());
    assert_eq!(
        drain(&mut first).await,
        vec![json(MESSAGE_DELTA), json(MESSAGE_STOP)]
    );
    assert_eq!(drain(&mut second).await, vec![json(INCOMPLETE_ERROR)]);
}

// The input's redactor scrubs the client-visible failure message.
#[tokio::test]
async fn the_redactor_scrubs_the_error_chunk() {
    let mut source = scripted(&[]);
    source.failure = Some(cause(
        r#"{"kind":"plain","text":"sk-1234567890abcdef boom"}"#,
    ));
    let mut stream = redacting_stream(source);
    assert_eq!(next(&mut stream).await, message_start());
    assert_eq!(
        next(&mut stream).await,
        json(r#"{"type":"error","error":{"type":"api_error","message":"REDACTED boom"}}"#)
    );
}

const MESSAGE_START_FRAME: &str = concat!(
    "event: message_start\ndata: ",
    r#"{"type": "message_start", "message": {"id": "msg_00000000-0000-0000-0000-000000000007", "type": "message", "role": "assistant", "content": [], "model": "model_a", "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 0, "output_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}}"#,
    "\n\n",
);
const MESSAGE_DELTA_FRAME: &str = concat!(
    "event: message_delta\ndata: ",
    r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"input_tokens": 0, "output_tokens": 0}}"#,
    "\n\n",
);
const MESSAGE_STOP_FRAME: &str = concat!(
    "event: message_stop\ndata: ",
    r#"{"type": "message_stop"}"#,
    "\n\n"
);
// The data line carries the six-character escape Python's json.dumps writes for a non-ASCII delta.
const NON_ASCII_FRAME: &str = concat!(
    "event: content_block_delta\ndata: ",
    r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": ""#,
    "\\",
    "u00e9",
    r#""}}"#,
    "\n\n",
);

fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .expect("the fixture time is RFC 3339"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}
