#![cfg(test)]

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::agents::json::AgentJson;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_core::Stream;
use indexmap::IndexMap;
use serde_json::Number;

use super::support::counter;
use betterlite_core::agents::provider::a2a::{IntegerId, JsonRpcId};

use crate::client::records::MessageCommon;
use crate::client::records::compat::{
    Message, MessageKind, Part, Role, SendMessageRequest, StatusUpdateKind, TaskState, TaskStatus,
    TaskStatusUpdate, TextPart,
};
use crate::client::records::response::{SendMessageResponse, StreamResult};
use crate::streaming_iterator::{
    A2AStreamingIterator, DEFAULT_MAX_RETAINED_BYTES, StreamSetup, StreamSuccessCall,
    StreamSuccessDispatch, StreamedChunk, TokenCounter, json_rpc_id_json,
};

const HELLO: &str = r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"message","messageId":"m2","role":"agent","parts":[{"kind":"text","text":"Hello"}]}}"#;
const HELLO_WORLD: &str = r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"message","messageId":"m3","role":"agent","parts":[{"kind":"text","text":"Hello world"}]}}"#;
const SECOND: &str = r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"message","messageId":"m4","role":"agent","parts":[{"kind":"text","text":"second"}]}}"#;
const COMPLETED: &str = r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"status-update","taskId":"t1","contextId":"c1","status":{"state":"completed"},"final":true}}"#;
const WORKING: &str = r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"status-update","taskId":"t1","contextId":"c1","status":{"state":"working"},"final":false}}"#;
const TASK: &str = r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"task","id":"t1","contextId":"c1","status":{"state":"working"}}}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WireChunk(AgentJson);

impl StreamedChunk for WireChunk {
    fn wire(&self) -> Result<AgentJson, serde_json::Error> {
        Ok(self.0.clone())
    }
}

#[derive(Clone, Copy)]
struct UnreadableChunk;

impl StreamedChunk for UnreadableChunk {
    fn wire(&self) -> Result<AgentJson, serde_json::Error> {
        Err(serde_json::Error::io(io::Error::other(
            "the chunk cannot be read",
        )))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamFailure;

struct ScriptedStream<Chunk> {
    items: VecDeque<Result<Chunk, StreamFailure>>,
}

impl<Chunk: Unpin> Stream for ScriptedStream<Chunk> {
    type Item = Result<Chunk, StreamFailure>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().items.pop_front())
    }
}

struct CharCounter;

#[async_trait]
impl TokenCounter for CharCounter {
    async fn count_tokens(&self, text: &str) -> usize {
        text.chars().count()
    }
}

#[derive(Clone)]
struct Recorded {
    call: StreamSuccessCall,
    details: IndexMap<String, Json>,
}

#[derive(Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<Recorded>>>,
}

#[async_trait]
impl StreamSuccessDispatch for Recorder {
    async fn dispatch(&mut self, logging: &mut LoggingState, call: StreamSuccessCall) {
        let details = logging.model_call_details.clone();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded { call, details });
    }
}

fn recorder() -> (Recorder, Arc<Mutex<Vec<Recorded>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Recorder {
            calls: Arc::clone(&calls),
        },
        calls,
    )
}

fn snapshot(calls: &Arc<Mutex<Vec<Recorded>>>) -> Vec<Recorded> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn fixed_time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("the fixture is RFC 3339")
}

fn clock(start_text: &str, end_text: &str) -> Box<dyn FnMut() -> DateTime<FixedOffset> + Send> {
    let start = fixed_time(start_text);
    let end = fixed_time(end_text);
    let mut times = VecDeque::from([start, end]);
    Box::new(move || times.pop_front().unwrap_or(end))
}

type TestClock = Box<dyn FnMut() -> DateTime<FixedOffset> + Send>;

fn setup<'counter>(
    dispatch: Recorder,
    counter: &'counter CharCounter,
    start: &str,
    end: &str,
) -> StreamSetup<'counter, Recorder, TestClock> {
    StreamSetup {
        counter,
        max_retained_bytes: DEFAULT_MAX_RETAINED_BYTES,
        dispatch,
        clock: clock(start, end),
    }
}

const START: &str = "2026-01-01T00:00:01+00:00";
const END: &str = "2026-01-01T00:00:02+00:00";

fn logging(details: &[(&str, Json)]) -> LoggingState {
    let mut context = CorrelationContext::default();
    let mut logging = LoggingState::new(
        LoggingInit {
            model: "a2a_agent/agent".to_owned(),
            call_type: "asend_message_streaming".to_owned(),
            start_time: fixed_time("2026-01-01T00:00:00+00:00"),
            litellm_call_id: "req-1".to_owned(),
            function_id: "req-1".to_owned(),
            messages: None,
            stream: Some(false),
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
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor compiles"),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("the fixture builds");
    let mut call_details = IndexMap::new();
    for (name, value) in details {
        call_details.insert((*name).to_owned(), value.clone());
    }
    logging.model_call_details = call_details;
    logging
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(params(pairs))
}

fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    values
}

fn priced() -> LoggingState {
    logging(&[("litellm_params", object(&[("cost_per_query", number(1))]))])
}

fn request_of(id: &str, parts: &str) -> SendMessageRequest {
    let source = format!(
        r#"{{"id":{id},"params":{{"message":{{"role":"user","parts":{parts},"messageId":"m1"}}}}}}"#
    );
    serde_json::from_str(&source).expect("the fixture request parses")
}

fn plain_request() -> SendMessageRequest {
    request_of(r#""req-1""#, "[]")
}

fn text_request(parts: &str) -> SendMessageRequest {
    request_of(r#""req-1""#, parts)
}

fn chunk(source: &str) -> WireChunk {
    WireChunk(serde_json::from_str(source).expect("the fixture chunk is JSON"))
}

fn chunks(sources: &[&str]) -> ScriptedStream<WireChunk> {
    ScriptedStream {
        items: sources.iter().map(|source| Ok(chunk(source))).collect(),
    }
}

fn failing_stream(sources: &[&str]) -> ScriptedStream<WireChunk> {
    let mut items = sources
        .iter()
        .map(|source| Ok(chunk(source)))
        .collect::<VecDeque<_>>();
    items.push_back(Err(StreamFailure));
    ScriptedStream { items }
}

async fn collect_chunks<Chunk, Source, Dispatch, Clock>(
    iterator: &mut A2AStreamingIterator<'_, Chunk, Source, Dispatch, Clock>,
) -> Vec<Result<Chunk, StreamFailure>>
where
    Chunk: StreamedChunk + Clone,
    Source: Stream<Item = Result<Chunk, StreamFailure>> + Unpin,
    Dispatch: StreamSuccessDispatch,
    Clock: FnMut() -> DateTime<FixedOffset>,
{
    let mut items = Vec::new();
    while let Some(item) = iterator.next_chunk().await {
        items.push(item);
    }
    items
}

fn result_object(result: &Json, key: &str) -> Option<Json> {
    match result {
        Json::Object(fields) => fields.get(key).cloned(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn completion_usage() -> Json {
    object(&[
        ("prompt_tokens", number(0)),
        ("completion_tokens", number(5)),
        ("total_tokens", number(5)),
    ])
}

fn chunk_result(source: &str) -> Json {
    serde_json::from_value(chunk(source).0["result"].clone()).expect("the result is JSON")
}

#[tokio::test]
async fn collects_every_chunk_and_extracts_its_text() {
    let (dispatch, _calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO, HELLO_WORLD, COMPLETED]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    let items = collect_chunks(&mut iterator).await;

    assert_eq!(items.len(), 3);
    assert!(items.iter().all(Result::is_ok));
    assert_eq!(iterator.chunks.len(), 3);
    assert_eq!(
        iterator.collected_text_parts.as_slice(),
        ["Hello", "Hello world"]
    );
    assert_eq!(
        iterator.final_chunk.as_ref().map(|item| item.0.clone()),
        Some(chunk(COMPLETED).0)
    );
}

#[tokio::test]
async fn the_completion_counts_only_the_last_extracted_text() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO, HELLO_WORLD]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    let recorded = snapshot(&calls);
    assert_eq!(
        recorded[0].details.get("usage"),
        Some(&object(&[
            ("prompt_tokens", number(0)),
            ("completion_tokens", number(11)),
            ("total_tokens", number(11)),
        ]))
    );
}

#[tokio::test]
async fn the_prompt_counts_the_request_message_text() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO]),
        text_request(r#"[{"kind":"text","text":"one"},{"kind":"text","text":"two"}]"#),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    let recorded = snapshot(&calls);
    assert_eq!(
        recorded[0].details.get("usage"),
        Some(&object(&[
            ("prompt_tokens", number(7)),
            ("completion_tokens", number(5)),
            ("total_tokens", number(12)),
        ]))
    );
}

#[tokio::test]
async fn non_text_parts_add_nothing_to_the_prompt() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO]),
        text_request(r#"[{"kind":"data","data":{"value":1}},{"kind":"text","text":"ok"}]"#),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    let recorded = snapshot(&calls);
    assert_eq!(
        recorded[0].details.get("usage"),
        Some(&object(&[
            ("prompt_tokens", number(2)),
            ("completion_tokens", number(5)),
            ("total_tokens", number(7)),
        ]))
    );
}

#[tokio::test]
async fn the_last_chunk_is_the_final_chunk_without_a_completion_marker() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO, SECOND]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    assert_eq!(
        iterator.final_chunk.as_ref().map(|item| item.0.clone()),
        Some(chunk(SECOND).0)
    );
    let recorded = snapshot(&calls);
    assert_eq!(
        result_object(&recorded[0].call.result, "result"),
        Some(chunk_result(SECOND))
    );
}

#[tokio::test]
async fn the_first_completed_chunk_stays_the_final_chunk() {
    let (dispatch, _calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO, COMPLETED, SECOND]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    assert_eq!(
        iterator.final_chunk.as_ref().map(|item| item.0.clone()),
        Some(chunk(COMPLETED).0)
    );
}

#[tokio::test]
async fn status_and_task_chunks_add_no_text() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[WORKING, COMPLETED, TASK]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    assert!(iterator.collected_text_parts.is_empty());
    assert_eq!(
        iterator.final_chunk.as_ref().map(|item| item.0.clone()),
        Some(chunk(COMPLETED).0)
    );
    let recorded = snapshot(&calls);
    assert_eq!(
        result_object(
            recorded[0].details.get("usage").expect("the usage is set"),
            "completion_tokens"
        ),
        Some(number(0))
    );
}

#[tokio::test]
async fn completion_sets_the_logging_details_and_the_dispatch_request() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO, COMPLETED]),
        plain_request(),
        logging(&[(
            "litellm_params",
            object(&[
                ("cost_per_query", number(1)),
                ("metadata", object(&[("user_id", text("u-1"))])),
            ]),
        )]),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    let recorded = snapshot(&calls);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].details.get("stream"), Some(&Json::Bool(false)));
    assert_eq!(recorded[0].details.get("response_cost"), Some(&number(1)));
    assert_eq!(recorded[0].details.get("usage"), Some(&completion_usage()));
    assert_eq!(
        recorded[0].details.get("litellm_params"),
        Some(&object(&[
            ("cost_per_query", number(1)),
            ("metadata", object(&[("user_id", text("u-1"))])),
        ]))
    );
    assert_eq!(recorded[0].call.start_time, fixed_time(START));
    assert_eq!(recorded[0].call.end_time, fixed_time(END));
    assert_eq!(recorded[0].call.cache_hit, None);
    assert!(recorded[0].call.prefer_async_handlers);
}

#[tokio::test]
async fn the_dispatch_result_carries_the_id_usage_and_final_result() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO, COMPLETED]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    let recorded = snapshot(&calls);
    assert_eq!(
        result_object(&recorded[0].call.result, "id"),
        Some(text("req-1"))
    );
    assert_eq!(
        result_object(&recorded[0].call.result, "jsonrpc"),
        Some(text("2.0"))
    );
    assert_eq!(
        result_object(&recorded[0].call.result, "usage"),
        Some(completion_usage())
    );
    assert_eq!(
        result_object(&recorded[0].call.result, "result"),
        Some(chunk_result(COMPLETED))
    );
}

#[tokio::test]
async fn the_completion_dispatches_only_once() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;
    assert!(iterator.next_chunk::<StreamFailure>().await.is_none());

    assert_eq!(snapshot(&calls).len(), 1);
}

#[tokio::test]
async fn an_empty_stream_dispatches_zero_usage_without_a_result() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    let items = collect_chunks(&mut iterator).await;

    assert!(items.is_empty());
    assert!(iterator.final_chunk.is_none());
    let recorded = snapshot(&calls);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        result_object(&recorded[0].call.result, "usage"),
        Some(object(&[
            ("prompt_tokens", number(0)),
            ("completion_tokens", number(0)),
            ("total_tokens", number(0)),
        ]))
    );
    assert_eq!(result_object(&recorded[0].call.result, "result"), None);
}

#[tokio::test]
async fn a_stream_error_is_propagated_without_completing() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        failing_stream(&[HELLO]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    assert!(
        iterator
            .next_chunk::<StreamFailure>()
            .await
            .is_some_and(|item| item.is_ok())
    );
    assert_eq!(
        iterator.next_chunk::<StreamFailure>().await,
        Some(Err(StreamFailure))
    );

    assert!(snapshot(&calls).is_empty());
    assert_eq!(iterator.logging.model_call_details.get("usage"), None);
}

#[tokio::test]
async fn an_accounting_failure_is_swallowed_and_skips_the_dispatch() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO]),
        plain_request(),
        logging(&[("litellm_params", text("not-an-object"))]),
        setup(dispatch, &counter, START, END),
    );

    let items = collect_chunks(&mut iterator).await;

    assert_eq!(items.len(), 1);
    assert!(snapshot(&calls).is_empty());
    assert_eq!(
        iterator.logging.model_call_details.get("stream"),
        Some(&Json::Bool(false))
    );
    assert!(iterator.logging.model_call_details.get("usage").is_some());
    assert_eq!(
        iterator.logging.model_call_details.get("response_cost"),
        None
    );
}

#[tokio::test]
async fn a_chunk_with_a_malformed_part_is_still_collected() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[
            r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"message","messageId":"m9","role":"agent","parts":null}}"#,
        ]),
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    assert_eq!(iterator.chunks.len(), 1);
    assert!(iterator.collected_text_parts.is_empty());
    let recorded = snapshot(&calls);
    assert_eq!(recorded.len(), 1);
    let malformed = object(&[
        ("kind", text("message")),
        ("messageId", text("m9")),
        ("role", text("agent")),
        ("parts", Json::Null),
    ]);
    assert_eq!(
        result_object(&recorded[0].call.result, "result"),
        Some(malformed)
    );
}

#[tokio::test]
async fn an_unreadable_final_chunk_dispatches_without_a_result() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let stream = ScriptedStream {
        items: VecDeque::from([Ok(UnreadableChunk)]),
    };
    let mut iterator = A2AStreamingIterator::new(
        stream,
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    let items = collect_chunks(&mut iterator).await;

    assert_eq!(items.len(), 1);
    assert_eq!(iterator.chunks.len(), 1);
    assert!(iterator.collected_text_parts.is_empty());
    let recorded = snapshot(&calls);
    assert_eq!(recorded.len(), 1);
    assert_eq!(result_object(&recorded[0].call.result, "result"), None);
}

#[tokio::test]
async fn token_rates_price_the_counts() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO_WORLD]),
        text_request(r#"[{"kind":"text","text":"ab"}]"#),
        logging(&[(
            "litellm_params",
            object(&[
                ("input_cost_per_token", number(2)),
                ("output_cost_per_token", number(3)),
            ]),
        )]),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    let recorded = snapshot(&calls);
    assert_eq!(recorded[0].details.get("response_cost"), Some(&number(37)));
}

#[tokio::test]
async fn the_result_keeps_an_integer_request_id() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut iterator = A2AStreamingIterator::new(
        chunks(&[HELLO]),
        request_of("7", "[]"),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    collect_chunks(&mut iterator).await;

    let recorded = snapshot(&calls);
    assert_eq!(
        result_object(&recorded[0].call.result, "id"),
        Some(number(7))
    );
}

#[test]
fn request_ids_keep_their_json_form() {
    let numeric_id =
        JsonRpcId::Integer(IntegerId::try_from(Number::from(7_u64)).expect("the id is an integer"));
    let text_id = JsonRpcId::String("req-1".to_owned());

    assert_eq!(json_rpc_id_json(&numeric_id), number(7));
    assert_eq!(json_rpc_id_json(&text_id), text("req-1"));
}

#[tokio::test]
async fn the_counter_adapter_counts_the_default_encoding_tokens() {
    let counter = counter();

    assert_eq!(counter.count_tokens("hello world").await, 2);
    assert_eq!(counter.count_tokens("").await, 0);
}

fn response_message(text: &str) -> SendMessageResponse {
    SendMessageResponse {
        jsonrpc: "2.0".to_owned(),
        id: JsonRpcId::String("req-1".to_owned()),
        result: StreamResult::Message(Message {
            kind: MessageKind::Message,
            role: Role::Agent,
            parts: vec![Part::Text(TextPart {
                text: text.to_owned(),
                metadata: None,
            })],
            common: MessageCommon {
                message_id: "m2".to_owned(),
                context_id: None,
                task_id: None,
                reference_task_ids: None,
                metadata: None,
                extensions: None,
            },
        }),
    }
}

fn completed_status() -> SendMessageResponse {
    SendMessageResponse {
        jsonrpc: "2.0".to_owned(),
        id: JsonRpcId::String("req-1".to_owned()),
        result: StreamResult::StatusUpdate(TaskStatusUpdate {
            kind: StatusUpdateKind::StatusUpdate,
            task_id: "t1".to_owned(),
            context_id: "c1".to_owned(),
            status: TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: None,
            },
            is_final: true,
            metadata: None,
        }),
    }
}

#[tokio::test]
async fn production_response_chunks_are_read_through_their_wire_form() {
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    assert!(
        response_message("Hello")
            .wire()
            .expect("the chunk serializes")
            .get("result")
            .is_some()
    );
    let stream = ScriptedStream {
        items: VecDeque::from([Ok(response_message("Hello")), Ok(completed_status())]),
    };
    let mut iterator = A2AStreamingIterator::new(
        stream,
        plain_request(),
        priced(),
        setup(dispatch, &counter, START, END),
    );

    let items = collect_chunks(&mut iterator).await;

    assert_eq!(items.len(), 2);
    assert!(items.iter().all(Result::is_ok));
    assert_eq!(iterator.collected_text_parts.as_slice(), ["Hello"]);
    assert!(matches!(
        iterator.final_chunk.as_ref().map(|chunk| &chunk.result),
        Some(StreamResult::StatusUpdate(_))
    ));
    assert_eq!(
        iterator
            .final_chunk
            .as_ref()
            .map(|chunk| chunk.wire().expect("the chunk serializes")),
        Some(chunk(COMPLETED).0)
    );
    let recorded = snapshot(&calls);
    assert_eq!(
        result_object(&recorded[0].call.result, "result"),
        Some(chunk_result(COMPLETED))
    );
}

fn numbered_message(index: usize, text: &str) -> String {
    format!(
        r#"{{"id":"req-1","jsonrpc":"2.0","result":{{"kind":"message","messageId":"m{index}","role":"agent","parts":[{{"kind":"text","text":"{text}"}}]}}}}"#
    )
}

fn retained_cost(source: &str, text: &str) -> usize {
    chunk(source).0.to_string().len().saturating_add(text.len())
}

#[tokio::test]
async fn chunks_past_the_retention_cap_are_forwarded_but_not_kept() {
    // every text has its own length, so the usage shows which text was counted
    let texts: Vec<String> = (1..=4).map(|length| "x".repeat(length)).collect();
    let sources: Vec<String> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| numbered_message(index, text))
        .collect();
    let cap = (0..3)
        .map(|index| retained_cost(&sources[index], &texts[index]))
        .sum();
    let (dispatch, calls) = recorder();
    let counter = CharCounter;
    let mut stream_setup = setup(dispatch, &counter, START, END);
    stream_setup.max_retained_bytes = cap;
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let mut iterator =
        A2AStreamingIterator::new(chunks(&refs), plain_request(), priced(), stream_setup);

    let items = collect_chunks(&mut iterator).await;

    let forwarded: Vec<AgentJson> = items
        .into_iter()
        .map(|item| item.expect("a chunk").0)
        .collect();
    let expected: Vec<AgentJson> = refs.iter().map(|source| chunk(source).0).collect();
    assert_eq!(forwarded, expected);
    assert_eq!(iterator.chunks.len(), 3);
    assert_eq!(iterator.collected_text_parts.len(), 3);
    assert_eq!(iterator.retained_bytes, cap);
    assert!(iterator.retention_capped);
    let recorded = snapshot(&calls);
    assert_eq!(
        recorded[0].details.get("usage"),
        Some(&object(&[
            ("prompt_tokens", number(0)),
            ("completion_tokens", number(4)),
            ("total_tokens", number(4)),
        ]))
    );
    assert_eq!(
        result_object(&recorded[0].call.result, "result"),
        Some(chunk_result(&sources[3]))
    );
}
