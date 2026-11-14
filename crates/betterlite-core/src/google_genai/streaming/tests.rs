#![cfg(test)]

use std::fs;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
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
use chrono::{DateTime, FixedOffset};
use futures_util::Stream;
use futures_util::stream::{BoxStream, iter};
use indexmap::IndexMap;

use super::logging::GenAiStreamLogging;
use super::metadata::GenAiStreamMetadata;
use super::provider::GenAiProvider;
use super::{AsyncGoogleGenAiStream, GenAiStreamError, GoogleGenAiStream};

const LINE_LIMIT: usize = 0x0010_0000;
const SMALL_LINE_LIMIT: usize = 8;
const NOW: &str = "2000-01-01T00:00:00+00:00";
const CORPUS_SEEDS: usize = 7;

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
        let recorded = RecordedCall {
            handler: self.name,
            url_route: call.url_route.to_owned(),
            request_body: call.request_body.cloned(),
            endpoint_type: call.endpoint_type,
            start_time: call.start_time,
            end_time: call.end_time,
            lines: call.lines.to_vec(),
            model: call.model.map(str::to_owned),
        };
        self.calls.lock().expect("the call log lock").push(recorded);
        Ok(HandlerOutcome::default())
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

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
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

fn logging_for(calls: Arc<Mutex<Vec<RecordedCall>>>) -> GenAiStreamLogging {
    let handlers = ProviderChunkHandlers {
        anthropic: Arc::new(RecordingHandler {
            name: "anthropic",
            calls: Arc::clone(&calls),
        }),
        vertex: Arc::new(RecordingHandler {
            name: "vertex",
            calls: Arc::clone(&calls),
        }),
        gemini: Arc::new(RecordingHandler {
            name: "gemini",
            calls: Arc::clone(&calls),
        }),
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
            slot: Mutex::new(None),
            dispatch: Arc::new(NoDispatch),
        }),
        passthrough: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
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
    let stream_metadata = metadata(GenAiProvider::Gemini);
    GoogleGenAiStream::new(chunks.into_iter().map(Ok), stream_metadata, LINE_LIMIT)
}

fn async_stream_from_source(
    source: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    stream_metadata: GenAiStreamMetadata,
    logging: Option<GenAiStreamLogging>,
) -> AsyncGoogleGenAiStream {
    AsyncGoogleGenAiStream::new(
        source,
        stream_metadata,
        logging,
        Arc::new(FixedClock),
        LINE_LIMIT,
    )
}

fn async_stream(
    chunks: Vec<Vec<u8>>,
    stream_metadata: GenAiStreamMetadata,
    logging: Option<GenAiStreamLogging>,
) -> AsyncGoogleGenAiStream {
    let source: BoxStream<'static, Result<Vec<u8>, HttpError>> =
        Box::pin(iter(chunks.into_iter().map(Ok)));
    async_stream_from_source(source, stream_metadata, logging)
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
    for event in &events {
        assert!(
            !event
                .windows(6)
                .any(|window| window == b"[DONE]".as_slice())
        );
    }
    assert_eq!(stream.collected_chunks, events);
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
    let payload = "QUJD".repeat(128 * 1024);
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
    let mut stream = GoogleGenAiStream::new(
        chunks.into_iter(),
        metadata(GenAiProvider::Gemini),
        LINE_LIMIT,
    );
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
    let mut stream = GoogleGenAiStream::new(
        chunks.into_iter(),
        metadata(GenAiProvider::Gemini),
        SMALL_LINE_LIMIT,
    );
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
        GenAiStreamError::Lines(AlephAlphaError::LineTooLong { limit: 8 })
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
    assert_eq!(stream.collected_chunks, events);
}

#[tokio::test]
async fn async_stream_routes_the_collected_events_to_the_gemini_handler_at_eof() {
    let calls = call_log();
    let mut stream = async_stream(
        vec![body(&["data: {\"a\":1}", "", "data: {\"b\":2}"])],
        metadata(GenAiProvider::Gemini),
        Some(logging_for(Arc::clone(&calls))),
    );
    let events = drain(&mut stream).await;
    assert_eq!(events.len(), 2);
    let logged = recorded_calls(&calls);
    assert_eq!(logged.len(), 1);
    let call = logged.first().expect("one call was recorded");
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
    assert_eq!(stream.collected_chunks, events);
}

#[tokio::test]
async fn async_stream_logs_the_request_body_it_was_given() {
    let calls = call_log();
    let mut stream_metadata = metadata(GenAiProvider::Gemini);
    stream_metadata
        .request_body
        .insert("contents".to_owned(), Json::Text("hi".to_owned()));
    let mut stream = async_stream(
        vec![body(&["data: a", ""])],
        stream_metadata,
        Some(logging_for(Arc::clone(&calls))),
    );
    assert_eq!(drain(&mut stream).await.len(), 1);
    let logged = recorded_calls(&calls);
    let call = logged.first().expect("one call was recorded");
    let expected = Json::Object(IndexMap::from([(
        "contents".to_owned(),
        Json::Text("hi".to_owned()),
    )]));
    assert_eq!(call.request_body, Some(expected));
}

#[tokio::test]
async fn async_stream_routes_a_vertex_provider_to_the_vertex_handler() {
    let calls = call_log();
    let mut stream = async_stream(
        vec![body(&["data: a", ""])],
        metadata(GenAiProvider::VertexAi),
        Some(logging_for(Arc::clone(&calls))),
    );
    assert_eq!(drain(&mut stream).await.len(), 1);
    let logged = recorded_calls(&calls);
    let call = logged.first().expect("one call was recorded");
    assert_eq!(call.handler, "vertex");
    assert_eq!(call.endpoint_type, EndpointType::VertexAi);
}

#[tokio::test]
async fn async_stream_logs_once_and_only_after_the_end() {
    let calls = call_log();
    let mut stream = async_stream(
        vec![body(&["data: a", ""])],
        metadata(GenAiProvider::Gemini),
        Some(logging_for(Arc::clone(&calls))),
    );
    let first = stream.next_event().await.expect("the first event");
    assert_eq!(first, Some(b"data: a\n\n".to_vec()));
    assert!(recorded_calls(&calls).is_empty());
    assert!(stream.next_event().await.expect("the end").is_none());
    assert_eq!(recorded_calls(&calls).len(), 1);
    assert!(stream.next_event().await.expect("the end again").is_none());
    assert_eq!(recorded_calls(&calls).len(), 1);
}

#[tokio::test]
async fn async_stream_logs_an_empty_stream_at_eof() {
    let calls = call_log();
    let mut stream = async_stream(
        Vec::new(),
        metadata(GenAiProvider::Gemini),
        Some(logging_for(Arc::clone(&calls))),
    );
    assert!(stream.next_event().await.expect("the end").is_none());
    let logged = recorded_calls(&calls);
    let call = logged.first().expect("one call was recorded");
    assert!(call.lines.is_empty());
    assert_eq!(call.request_body, Some(Json::Object(IndexMap::new())));
}

#[tokio::test]
async fn async_stream_logs_nothing_after_a_transport_failure() {
    let calls = call_log();
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        vec![Ok(b"data: a\n\ndata: b".to_vec()), Err(HttpError::Timeout)];
    let source: BoxStream<'static, Result<Vec<u8>, HttpError>> = Box::pin(iter(chunks));
    let mut stream = async_stream_from_source(
        source,
        metadata(GenAiProvider::Gemini),
        Some(logging_for(Arc::clone(&calls))),
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
    assert!(recorded_calls(&calls).is_empty());
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

#[tokio::test]
async fn replays_the_genai_sse_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/genai_sse");
    let mut seeds: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in fs::read_dir(corpus).expect("the corpus folder") {
        let entry = entry.expect("a corpus entry");
        let bytes = fs::read(entry.path()).expect("a seed file");
        let name = entry
            .file_name()
            .to_str()
            .expect("the seed name is UTF-8")
            .to_owned();
        seeds.push((name, bytes));
    }
    seeds.sort();
    assert_eq!(seeds.len(), CORPUS_SEEDS);
    for (name, bytes) in seeds {
        let whole = replay(vec![bytes.clone()]);
        let byte_wise: Vec<Vec<u8>> = bytes.chunks(1).map(<[u8]>::to_vec).collect();
        assert_eq!(
            replay(byte_wise),
            whole,
            "{name} parses the same byte by byte"
        );
        let mut stream = async_stream(vec![bytes.clone()], metadata(GenAiProvider::Gemini), None);
        assert_eq!(
            drain(&mut stream).await,
            whole,
            "{name} parses the same asynchronously"
        );
        for event in &whole {
            assert!(
                event.ends_with(b"\n\n"),
                "{name} ends every event with a blank line"
            );
        }
        check_corpus_expectation(&name, &bytes, &whole);
    }
}

fn replay(chunks: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut stream = sync_stream(chunks);
    sync_events(&mut stream)
}

fn check_corpus_expectation(name: &str, bytes: &[u8], events: &[Vec<u8>]) {
    match name {
        "blank-line-delimiter" => assert_eq!(
            events,
            [
                b"data: {\"a\":1}\n\n".to_vec(),
                b"data: {\"b\":2}\n\n".to_vec()
            ]
        ),
        "crlf-lines" => assert_eq!(events, [b"data: {\"a\":1}\n\n".to_vec()]),
        "empty" => assert!(events.is_empty()),
        "invalid-utf8" => check_replacement_decoding(events),
        "large-inline-media" => assert_eq!(events, [bytes.to_vec()]),
        "split-utf8" => assert_eq!(events, [bytes.to_vec()]),
        "unterminated-final-event" => assert_eq!(
            events,
            [
                b"data: {\"a\":1}\n\n".to_vec(),
                b"data: {\"b\":2}\n\n".to_vec()
            ]
        ),
        other => panic!("the corpus seed {other} has no expectation"),
    }
}

fn check_replacement_decoding(events: &[Vec<u8>]) {
    let [event] = events else {
        panic!("the invalid UTF-8 seed yields one event");
    };
    assert!(event.starts_with(b"data: "));
    assert!(event.ends_with(b"\n\n"));
    assert!(
        event
            .windows(3)
            .any(|window| window == b"\xEF\xBF\xBD".as_slice())
    );
}
