#![cfg(test)]

mod corpus;

use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::router::{
    AnthropicFrameProbe, StreamLoggingRouter,
};
use betterlite_logging::passthrough_stream_logging::tinyfish::{
    TinyfishCollectedChunks, TinyfishStreamBilling,
};
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, EndpointType, HandlerError, HandlerOutcome,
    ProviderChunkHandlers,
};
use betterlite_models::decisions::Json;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use futures_util::Stream;
use futures_util::stream::{BoxStream, iter};
use indexmap::IndexMap;
use tokio::sync::{Notify, oneshot};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use super::{AsyncGoogleGenAiStream, GenAiStreamError, GoogleGenAiStream};
use crate::google_genai::logging::GenAiStreamLogging;
use crate::google_genai::metadata::GenAiStreamMetadata;
use crate::google_genai::provider::GenAiProvider;

const LINE_LIMIT: usize = 0x0010_0000;
const SMALL_LINE_LIMIT: usize = 9;
const NOW: &str = "2000-01-01T00:00:00+00:00";
const LOG_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
struct RecordedCall {
    handler: &'static str,
    url_route: String,
    request_body: Option<Json>,
    endpoint_type: EndpointType,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    lines: Vec<String>,
    model: Option<String>,
}

struct RecordingHandler {
    name: &'static str,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl CollectedChunksHandler for RecordingHandler {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        self.calls
            .lock()
            .expect("the call log lock")
            .push(recorded(self.name, call));
        Ok(HandlerOutcome::default())
    }
}

fn recorded(name: &'static str, call: &CollectedChunksCall<'_>) -> RecordedCall {
    RecordedCall {
        handler: name,
        url_route: call.url_route.to_owned(),
        request_body: call.request_body.cloned(),
        endpoint_type: call.endpoint_type,
        start_time: call.start_time,
        end_time: call.end_time,
        lines: call.lines.to_vec(),
        model: call.model.map(str::to_owned),
    }
}

// the collected chunks build runs on a blocking thread, so the handler can hold it until the test lets it go
struct BlockingHandler {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    gate: Arc<Gate>,
    started: Mutex<Option<oneshot::Sender<()>>>,
    done: Mutex<Option<oneshot::Sender<()>>>,
}

impl CollectedChunksHandler for BlockingHandler {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        self.calls
            .lock()
            .expect("the call log lock")
            .push(recorded("gemini", call));
        let started = self
            .started
            .lock()
            .expect("the started lock")
            .take()
            .expect("the test waits for the handler");
        started
            .send(())
            .expect("the test holds the started receiver");
        self.gate.wait();
        let done = self
            .done
            .lock()
            .expect("the done lock")
            .take()
            .expect("the test waits for the handler");
        done.send(()).expect("the test holds the done receiver");
        Ok(HandlerOutcome::default())
    }
}

#[derive(Default)]
struct Gate {
    opened: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn open(&self) {
        let mut opened = self.opened.lock().expect("the gate lock");
        *opened = true;
        drop(opened);
        self.changed.notify_all();
    }

    fn wait(&self) {
        let locked = self.opened.lock().expect("the gate lock");
        let opened = self
            .changed
            .wait_while(locked, |opened| !*opened)
            .expect("the gate lock");
        drop(opened);
    }
}

struct NoFrames;

impl AnthropicFrameProbe for NoFrames {
    fn is_message_stop(&self, _frames: &[u8]) -> bool {
        false
    }

    fn is_provider_error(&self, _frames: &[u8]) -> bool {
        false
    }
}

struct NoTinyfish;

#[async_trait]
impl TinyfishStreamBilling for NoTinyfish {
    fn sse_poller_spawned(&self, _logging_obj: &StreamLoggingObject) -> bool {
        false
    }

    fn run_id_from_sse_frames(&self, _frames: &[u8]) -> Option<String> {
        None
    }

    fn start_sse_run_billing(
        &self,
        _run_id: &str,
        _logging_obj: &StreamLoggingObject,
        _start_time: DateTime<FixedOffset>,
    ) {
    }

    async fn handle_logging_tinyfish_collected_chunks(
        &self,
        _call: &TinyfishCollectedChunks<'_>,
    ) -> Result<HandlerOutcome, HandlerError> {
        Ok(HandlerOutcome::default())
    }
}

struct RecordingDispatch {
    done: Arc<Notify>,
}

#[async_trait]
impl StreamSuccessDispatch for RecordingDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {
        self.done.notify_one();
    }
}

struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(NOW).expect("the fixed instant is RFC 3339")
    }
}

fn call_log() -> Arc<Mutex<Vec<RecordedCall>>> {
    Arc::new(Mutex::new(Vec::new()))
}

fn recorded_calls(calls: &Mutex<Vec<RecordedCall>>) -> Vec<RecordedCall> {
    calls.lock().expect("the call log lock").clone()
}

fn logging_for(calls: Arc<Mutex<Vec<RecordedCall>>>) -> (GenAiStreamLogging, Arc<Notify>) {
    let done = Arc::new(Notify::new());
    let gemini = Arc::new(RecordingHandler {
        name: "gemini",
        calls: Arc::clone(&calls),
    });
    let logging = logging_with_gemini(gemini, calls, Arc::clone(&done));
    (logging, done)
}

fn logging_with_gemini(
    gemini: Arc<dyn CollectedChunksHandler>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    done: Arc<Notify>,
) -> GenAiStreamLogging {
    let handlers = ProviderChunkHandlers {
        anthropic: Arc::new(RecordingHandler {
            name: "anthropic",
            calls: Arc::clone(&calls),
        }),
        vertex: Arc::new(RecordingHandler {
            name: "vertex",
            calls: Arc::clone(&calls),
        }),
        gemini,
        openai: Arc::new(RecordingHandler {
            name: "openai",
            calls,
        }),
    };
    let router = StreamLoggingRouter {
        providers: Arc::new(handlers),
        frames: Arc::new(NoFrames),
        tinyfish: Arc::new(NoTinyfish),
    };
    GenAiStreamLogging {
        router: Arc::new(router),
        logging_obj: Arc::new(StreamLoggingObject {
            slot: Mutex::new(Some(stream_logging_state())),
            dispatch: Arc::new(RecordingDispatch { done }),
        }),
        passthrough: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
        tasks: TaskTracker::new(),
    }
}

// the success dispatch only runs while the logging object still holds its state
fn stream_logging_state() -> StreamLoggingState {
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let redactor = Redactor::new(16).expect("the redactor builds");
    let logging = LoggingState::new(
        logging_init(),
        setup,
        &redactor,
        || "trace-1".to_owned(),
        || Duration::ZERO,
    )
    .expect("the logging state builds");
    StreamLoggingState {
        logging,
        correlation,
    }
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: "gemini-2.5-flash".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: FixedClock.now(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(true),
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
    }
}

fn metadata(provider: GenAiProvider) -> GenAiStreamMetadata {
    GenAiStreamMetadata {
        model: "gemini-2.5-flash".to_owned(),
        provider,
        request_body: IndexMap::new(),
        hidden_params: IndexMap::new(),
    }
}

fn body(lines: &[&str]) -> Vec<u8> {
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    body.into_bytes()
}

fn sync_stream(chunks: Vec<Vec<u8>>) -> GoogleGenAiStream {
    GoogleGenAiStream::new(chunks.into_iter().map(Ok), LINE_LIMIT)
}

fn async_stream_from_source(
    source: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    stream_metadata: GenAiStreamMetadata,
    logging: Option<GenAiStreamLogging>,
    line_limit: usize,
) -> AsyncGoogleGenAiStream {
    AsyncGoogleGenAiStream::new(
        source,
        stream_metadata,
        logging,
        Arc::new(FixedClock),
        line_limit,
    )
}

fn async_stream(
    chunks: Vec<Vec<u8>>,
    stream_metadata: GenAiStreamMetadata,
    logging: Option<GenAiStreamLogging>,
) -> AsyncGoogleGenAiStream {
    let source: BoxStream<'static, Result<Vec<u8>, HttpError>> =
        Box::pin(iter(chunks.into_iter().map(Ok)));
    async_stream_from_source(source, stream_metadata, logging, LINE_LIMIT)
}

fn sync_events(stream: &mut GoogleGenAiStream) -> Vec<Vec<u8>> {
    let collected: Result<Vec<Vec<u8>>, GenAiStreamError> = stream.by_ref().collect();
    collected.expect("the stream parses")
}

async fn drain(stream: &mut AsyncGoogleGenAiStream) -> Vec<Vec<u8>> {
    let mut events = Vec::new();
    while let Some(event) = stream.next_event().await.expect("the stream parses") {
        events.push(event);
    }
    events
}

#[test]
fn sync_stream_emits_events_verbatim_without_a_done_marker() {
    let chunks = vec![body(&[
        "data: {\"text\":\"Hello\"}",
        "",
        "data: {\"text\":\" world\"}",
        "",
    ])];
    let mut stream = sync_stream(chunks);
    let events = sync_events(&mut stream);
    assert_eq!(
        events,
        [
            b"data: {\"text\":\"Hello\"}\n\n".to_vec(),
            b"data: {\"text\":\" world\"}\n\n".to_vec(),
        ]
    );
    assert!(events.iter().all(|event| {
        !event
            .windows(6)
            .any(|window| window == b"[DONE]".as_slice())
    }));
    assert_eq!(stream.collected_chunks(), events.as_slice());
}

#[test]
fn sync_stream_joins_the_lines_of_one_event_with_line_feeds() {
    let mut stream = sync_stream(vec![body(&["data: first", "data: second", ""])]);
    let events = sync_events(&mut stream);
    assert_eq!(events, [b"data: first\ndata: second\n\n".to_vec()]);
}

#[test]
fn sync_stream_skips_blank_lines_that_open_no_event() {
    let mut stream = sync_stream(vec![body(&["", "", "data: a", "", "", "data: b", ""])]);
    let events = sync_events(&mut stream);
    assert_eq!(events, [b"data: a\n\n".to_vec(), b"data: b\n\n".to_vec()]);
}

#[test]
fn sync_stream_resets_the_event_size_after_each_event() {
    let mut stream = GoogleGenAiStream::new(
        vec![Ok(body(&["data: a", "", "data: b", ""]))].into_iter(),
        SMALL_LINE_LIMIT,
    );
    let events = sync_events(&mut stream);
    assert_eq!(events, [b"data: a\n\n".to_vec(), b"data: b\n\n".to_vec()]);
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_emits_the_event_left_open_at_end_of_input() {
    let mut stream = sync_stream(vec![b"data: {\"a\":1}\n\ndata: {\"b\":2}".to_vec()]);
    let events = sync_events(&mut stream);
    assert_eq!(
        events,
        [
            b"data: {\"a\":1}\n\n".to_vec(),
            b"data: {\"b\":2}\n\n".to_vec(),
        ]
    );
}

#[test]
fn sync_stream_keeps_a_large_inline_media_event_whole() {
    let payload = "QUJD".repeat(0x0002_0000);
    let line =
        format!("data: {{\"inlineData\":{{\"mimeType\":\"image/png\",\"data\":\"{payload}\"}}}}");
    let bytes = body(&[line.as_str(), ""]);
    let chunks: Vec<Vec<u8>> = bytes.chunks(7).map(<[u8]>::to_vec).collect();
    let mut stream = sync_stream(chunks);
    let events = sync_events(&mut stream);
    assert_eq!(events, [bytes]);
    assert!(
        events[0]
            .windows(payload.len())
            .any(|window| window == payload.as_bytes())
    );
}

#[test]
fn sync_stream_reassembles_a_utf8_sequence_split_across_reads() {
    let bytes = body(&["data: {\"text\":\"héllo 日本 🚀\"}", ""]);
    let chunks: Vec<Vec<u8>> = bytes.chunks(1).map(<[u8]>::to_vec).collect();
    let mut stream = sync_stream(chunks);
    assert_eq!(sync_events(&mut stream), [bytes]);
}

#[test]
fn sync_stream_stops_at_a_transport_failure() {
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        vec![Ok(b"data: a\n\ndata: b".to_vec()), Err(HttpError::Timeout)];
    let mut stream = GoogleGenAiStream::new(chunks.into_iter(), LINE_LIMIT);
    let first = stream
        .next()
        .expect("the first event")
        .expect("the first event parses");
    assert_eq!(first, b"data: a\n\n".to_vec());
    assert!(matches!(
        stream.next(),
        Some(Err(GenAiStreamError::Transport { .. }))
    ));
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_reports_a_long_line_after_the_events_before_it() {
    let input = b"data: a\n\ndata: 0123456789\n\n";
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        input.chunks(1).map(|byte| Ok(byte.to_vec())).collect();
    let mut stream = GoogleGenAiStream::new(chunks.into_iter(), SMALL_LINE_LIMIT);
    let first = stream
        .next()
        .expect("the first event")
        .expect("the first event parses");
    assert_eq!(first, b"data: a\n\n".to_vec());
    let failure = stream
        .next()
        .expect("the limit error")
        .expect_err("the long line is refused");
    assert!(matches!(
        failure,
        GenAiStreamError::Lines(AlephAlphaError::LineTooLong {
            limit: SMALL_LINE_LIMIT
        })
    ));
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_refuses_a_long_line_inside_one_read() {
    let mut stream = GoogleGenAiStream::new(
        vec![Ok(b"data: 0123456789\n\n".to_vec())].into_iter(),
        SMALL_LINE_LIMIT,
    );
    let failure = stream
        .next()
        .expect("the limit error")
        .expect_err("the long line is refused");
    assert!(matches!(
        failure,
        GenAiStreamError::Lines(AlephAlphaError::LineTooLong {
            limit: SMALL_LINE_LIMIT
        })
    ));
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_stops_at_an_over_limit_line_before_reading_more() {
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        vec![Ok(b"data: 0123456789".to_vec()), Err(HttpError::Timeout)];
    let mut stream = GoogleGenAiStream::new(chunks.into_iter(), SMALL_LINE_LIMIT);
    let failure = stream
        .next()
        .expect("the limit error")
        .expect_err("the open line is refused");
    assert!(matches!(
        failure,
        GenAiStreamError::Lines(AlephAlphaError::LineTooLong {
            limit: SMALL_LINE_LIMIT
        })
    ));
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_refuses_a_line_exactly_at_the_limit_as_an_event() {
    let mut stream = GoogleGenAiStream::new(
        vec![Ok(body(&["data: 012", ""]))].into_iter(),
        SMALL_LINE_LIMIT,
    );
    let failure = stream
        .next()
        .expect("the limit error")
        .expect_err("the event is refused");
    assert!(matches!(
        failure,
        GenAiStreamError::EventTooLong {
            limit: SMALL_LINE_LIMIT
        }
    ));
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_refuses_an_event_one_byte_past_the_limit() {
    let mut stream = GoogleGenAiStream::new(
        vec![Ok(body(&["data: ab", ""]))].into_iter(),
        SMALL_LINE_LIMIT,
    );
    let failure = stream
        .next()
        .expect("the limit error")
        .expect_err("the event is refused");
    assert!(matches!(
        failure,
        GenAiStreamError::EventTooLong {
            limit: SMALL_LINE_LIMIT
        }
    ));
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_refuses_an_event_that_grows_past_the_limit() {
    let input = body(&["data: a", "data: b"]);
    let mut stream = GoogleGenAiStream::new(vec![Ok(input)].into_iter(), SMALL_LINE_LIMIT);
    let failure = stream
        .next()
        .expect("the limit error")
        .expect_err("the growing event is refused");
    assert!(matches!(
        failure,
        GenAiStreamError::EventTooLong {
            limit: SMALL_LINE_LIMIT
        }
    ));
    assert!(stream.next().is_none());
}

#[test]
fn async_stream_rejects_a_sync_pull_with_not_implemented() {
    let mut stream = async_stream(Vec::new(), metadata(GenAiProvider::Gemini), None);
    let rejected = stream
        .next()
        .expect("the pull is refused")
        .expect_err("sync iteration is refused");
    assert_eq!(
        rejected.to_string(),
        "Use GoogleGenAIGenerateContentStreamingIterator for sync iteration"
    );
    assert!(stream.next().is_none());
}

#[test]
fn sync_stream_rejects_an_async_pull_with_litellms_message() {
    let mut stream = sync_stream(vec![body(&["data: a", ""])]);
    let mut context = Context::from_waker(Waker::noop());
    let rejected = Stream::poll_next(Pin::new(&mut stream), &mut context);
    let Poll::Ready(Some(Err(rejected))) = rejected else {
        panic!("the async pull is refused");
    };
    assert_eq!(
        rejected.to_string(),
        "Use AsyncGoogleGenAIGenerateContentStreamingIterator for async iteration"
    );
    assert!(matches!(
        Stream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(None)
    ));
    assert_eq!(sync_events(&mut stream), [b"data: a\n\n".to_vec()]);
}

#[tokio::test]
async fn async_stream_yields_the_same_events_and_collects_them() {
    let chunks = vec![body(&["data: {\"a\":1}", "", "data: {\"b\":2}"])];
    let expected = sync_events(&mut sync_stream(chunks.clone()));
    let mut stream = async_stream(chunks, metadata(GenAiProvider::Gemini), None);
    let events = drain(&mut stream).await;
    assert_eq!(events, expected);
    assert_eq!(
        events,
        [
            b"data: {\"a\":1}\n\n".to_vec(),
            b"data: {\"b\":2}\n\n".to_vec(),
        ]
    );
    assert_eq!(stream.collected_chunks(), events.as_slice());
}

#[tokio::test]
async fn async_stream_returns_none_after_the_last_event() {
    let mut stream = async_stream(
        vec![body(&["data: a", ""])],
        metadata(GenAiProvider::Gemini),
        None,
    );
    let first = stream
        .next_event()
        .await
        .expect("the first event parses")
        .expect("the first event arrives");
    assert_eq!(first, b"data: a\n\n".to_vec());
    assert!(stream.next_event().await.expect("the end parses").is_none());
    assert_eq!(
        stream.collected_chunks(),
        [b"data: a\n\n".to_vec()].as_slice()
    );
}

#[tokio::test]
async fn async_stream_routes_the_collected_events_to_the_gemini_handler_at_eof() {
    let calls = call_log();
    let (logging, logged) = logging_for(Arc::clone(&calls));
    let mut stream = async_stream(
        vec![body(&["data: {\"a\":1}", "", "data: {\"b\":2}"])],
        metadata(GenAiProvider::Gemini),
        Some(logging),
    );
    let events = drain(&mut stream).await;
    timeout(LOG_WAIT, logged.notified())
        .await
        .expect("the completion log runs");
    assert_eq!(events.len(), 2);
    let calls_recorded = recorded_calls(&calls);
    assert_eq!(calls_recorded.len(), 1);
    let call = calls_recorded.first().expect("one call was recorded");
    assert_eq!(call.handler, "gemini");
    assert_eq!(call.url_route, "/v1/generateContent");
    assert_eq!(call.endpoint_type, EndpointType::Gemini);
    assert_eq!(call.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(call.request_body, Some(Json::Object(IndexMap::new())));
    assert_eq!(
        call.lines,
        ["data: {\"a\":1}".to_owned(), "data: {\"b\":2}".to_owned()]
    );
    assert_eq!(call.start_time, FixedClock.now());
    assert_eq!(call.end_time, FixedClock.now());
    assert_eq!(stream.collected_chunks(), events.as_slice());
}

#[tokio::test]
async fn async_stream_logs_the_request_body_it_was_given() {
    let calls = call_log();
    let mut stream_metadata = metadata(GenAiProvider::Gemini);
    stream_metadata
        .request_body
        .insert("contents".to_owned(), Json::Text("hi".to_owned()));
    let (logging, logged) = logging_for(Arc::clone(&calls));
    let mut stream = async_stream(vec![body(&["data: a", ""])], stream_metadata, Some(logging));
    assert_eq!(drain(&mut stream).await.len(), 1);
    timeout(LOG_WAIT, logged.notified())
        .await
        .expect("the completion log runs");
    let calls_recorded = recorded_calls(&calls);
    let call = calls_recorded.first().expect("one call was recorded");
    let expected = Json::Object(IndexMap::from([(
        "contents".to_owned(),
        Json::Text("hi".to_owned()),
    )]));
    assert_eq!(call.request_body, Some(expected));
}

#[tokio::test]
async fn async_stream_routes_a_vertex_provider_to_the_vertex_handler() {
    let calls = call_log();
    let (logging, logged) = logging_for(Arc::clone(&calls));
    let mut stream = async_stream(
        vec![body(&["data: a", ""])],
        metadata(GenAiProvider::VertexAi),
        Some(logging),
    );
    assert_eq!(drain(&mut stream).await.len(), 1);
    timeout(LOG_WAIT, logged.notified())
        .await
        .expect("the completion log runs");
    let calls_recorded = recorded_calls(&calls);
    let call = calls_recorded.first().expect("one call was recorded");
    assert_eq!(call.handler, "vertex");
    assert_eq!(call.endpoint_type, EndpointType::VertexAi);
}

#[tokio::test]
async fn async_stream_logs_once_and_only_after_the_end() {
    let calls = call_log();
    let (logging, logged) = logging_for(Arc::clone(&calls));
    let mut stream = async_stream(
        vec![body(&["data: a", ""])],
        metadata(GenAiProvider::Gemini),
        Some(logging),
    );
    let first = stream.next_event().await.expect("the first event");
    assert_eq!(first, Some(b"data: a\n\n".to_vec()));
    assert!(recorded_calls(&calls).is_empty());
    assert!(stream.next_event().await.expect("the end").is_none());
    timeout(LOG_WAIT, logged.notified())
        .await
        .expect("the completion log runs");
    assert_eq!(recorded_calls(&calls).len(), 1);
    assert!(stream.next_event().await.expect("the end again").is_none());
    assert!(
        timeout(Duration::from_millis(200), logged.notified())
            .await
            .is_err(),
        "the completion is not logged twice"
    );
    assert_eq!(recorded_calls(&calls).len(), 1);
}

#[tokio::test]
async fn async_stream_logs_an_empty_stream_at_eof() {
    let calls = call_log();
    let (logging, logged) = logging_for(Arc::clone(&calls));
    let mut stream = async_stream(Vec::new(), metadata(GenAiProvider::Gemini), Some(logging));
    assert!(stream.next_event().await.expect("the end").is_none());
    timeout(LOG_WAIT, logged.notified())
        .await
        .expect("the completion log runs");
    let calls_recorded = recorded_calls(&calls);
    let call = calls_recorded.first().expect("one call was recorded");
    assert!(call.lines.is_empty());
    assert_eq!(call.request_body, Some(Json::Object(IndexMap::new())));
}

#[tokio::test]
async fn async_stream_logs_nothing_after_a_transport_failure() {
    let calls = call_log();
    let (logging, logged) = logging_for(Arc::clone(&calls));
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        vec![Ok(b"data: a\n\ndata: b".to_vec()), Err(HttpError::Timeout)];
    let source: BoxStream<'static, Result<Vec<u8>, HttpError>> = Box::pin(iter(chunks));
    let mut stream = async_stream_from_source(
        source,
        metadata(GenAiProvider::Gemini),
        Some(logging),
        LINE_LIMIT,
    );
    let first = stream.next_event().await.expect("the first event");
    assert_eq!(first, Some(b"data: a\n\n".to_vec()));
    assert!(matches!(
        stream.next_event().await,
        Err(GenAiStreamError::Transport { .. })
    ));
    assert!(
        stream
            .next_event()
            .await
            .expect("the stream is over")
            .is_none()
    );
    assert!(
        timeout(Duration::from_millis(200), logged.notified())
            .await
            .is_err(),
        "a failed stream is never logged"
    );
    assert!(recorded_calls(&calls).is_empty());
}

#[tokio::test]
async fn async_stream_logs_nothing_after_a_line_limit_failure() {
    let calls = call_log();
    let (logging, logged) = logging_for(Arc::clone(&calls));
    let chunks: Vec<Result<Vec<u8>, HttpError>> = vec![
        Ok(b"data: a\n\n".to_vec()),
        Ok(b"data: 0123456789\n\n".to_vec()),
    ];
    let source: BoxStream<'static, Result<Vec<u8>, HttpError>> = Box::pin(iter(chunks));
    let mut stream = async_stream_from_source(
        source,
        metadata(GenAiProvider::Gemini),
        Some(logging),
        SMALL_LINE_LIMIT,
    );
    let first = stream.next_event().await.expect("the first event");
    assert_eq!(first, Some(b"data: a\n\n".to_vec()));
    let failure = stream
        .next_event()
        .await
        .expect_err("the long line is refused");
    assert!(matches!(
        failure,
        GenAiStreamError::Lines(AlephAlphaError::LineTooLong {
            limit: SMALL_LINE_LIMIT
        })
    ));
    let over = stream.next_event().await;
    assert!(over.expect("the stream is over").is_none());
    assert!(
        timeout(Duration::from_millis(200), logged.notified())
            .await
            .is_err(),
        "a stream that failed on a long line is never logged"
    );
    assert!(recorded_calls(&calls).is_empty());
}

#[tokio::test]
async fn async_stream_returns_the_end_before_the_completion_log_finishes() {
    let calls = call_log();
    let gate = Arc::new(Gate::default());
    let (started_tx, started_rx) = oneshot::channel();
    let (done_tx, mut done_rx) = oneshot::channel();
    let handler = Arc::new(BlockingHandler {
        calls: Arc::clone(&calls),
        gate: Arc::clone(&gate),
        started: Mutex::new(Some(started_tx)),
        done: Mutex::new(Some(done_tx)),
    });
    let dispatched = Arc::new(Notify::new());
    let logging = logging_with_gemini(handler, Arc::clone(&calls), Arc::clone(&dispatched));
    let mut stream = async_stream(
        vec![body(&["data: a", ""])],
        metadata(GenAiProvider::Gemini),
        Some(logging),
    );
    let event = stream.next_event().await.expect("the event parses");
    assert_eq!(event, Some(b"data: a\n\n".to_vec()));
    let end = timeout(LOG_WAIT, stream.next_event())
        .await
        .expect("the end of the stream does not wait for the completion log")
        .expect("the end parses");
    assert!(end.is_none());
    timeout(LOG_WAIT, started_rx)
        .await
        .expect("the handler starts")
        .expect("the handler started");
    assert!(done_rx.try_recv().is_err(), "the handler is still blocked");
    drop(stream);
    gate.open();
    timeout(LOG_WAIT, dispatched.notified())
        .await
        .expect("the success dispatch runs after the stream was dropped");
    timeout(LOG_WAIT, done_rx)
        .await
        .expect("the log finishes")
        .expect("the log finished");
    let calls_recorded = recorded_calls(&calls);
    assert_eq!(calls_recorded.len(), 1);
    let call = calls_recorded.first().expect("one call was recorded");
    assert_eq!(call.handler, "gemini");
}

#[test]
fn provider_spelling_maps_only_gemini_to_the_gemini_endpoint() {
    assert_eq!(
        GenAiProvider::from_spelling("gemini").endpoint_type(),
        EndpointType::Gemini
    );
    assert_eq!(
        GenAiProvider::from_spelling("vertex_ai").endpoint_type(),
        EndpointType::VertexAi
    );
    assert_eq!(
        GenAiProvider::from_spelling("Vertex").endpoint_type(),
        EndpointType::VertexAi
    );
}
