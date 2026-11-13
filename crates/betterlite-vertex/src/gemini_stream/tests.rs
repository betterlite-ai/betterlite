#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::header::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;
use uuid::Uuid;

use super::adapters::{
    AsyncFragmentReader, FragmentReader, GeminiStream, ResponseClose, StreamFuture,
};
use super::buffer::MAX_RETAINED_BYTES;
use super::chunk::{Choice, StreamChunk};
use super::error::StreamError;
use super::{GeminiStreamDecoder, StreamOptions};
use crate::gemini_response_parts::projection::CallMode;

const FIXED: &str = "0123456789abcdef0123456789abcdef";
const NOW: i64 = 1_700_000_000;

fn fixed_ids() -> Box<dyn FnMut() -> Uuid + Send> {
    Box::new(|| Uuid::parse_str(FIXED).unwrap())
}

fn options() -> StreamOptions {
    StreamOptions {
        now: NOW,
        call_mode: CallMode::Tools,
    }
}

fn headers_map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn decoder_with(headers: &[(&str, &str)], call_mode: CallMode) -> GeminiStreamDecoder {
    GeminiStreamDecoder::new(
        StreamOptions {
            now: NOW,
            call_mode,
        },
        &headers_map(headers),
        fixed_ids(),
    )
}

fn decoder() -> GeminiStreamDecoder {
    decoder_with(&[], CallMode::Tools)
}

fn content(chunk: &StreamChunk) -> Option<&str> {
    match chunk.choices.first()? {
        Choice::Projected(choice) => choice.delta.content.as_deref(),
        Choice::EmptyDelta { .. } => None,
    }
}

fn finish_reason(chunk: &StreamChunk) -> Option<&'static str> {
    match chunk.choices.first()? {
        Choice::Projected(choice) => Some(choice.finish_reason),
        Choice::EmptyDelta { .. } => None,
    }
}

fn finish_reason_at(chunk: &StreamChunk, index: usize) -> Option<&'static str> {
    match chunk.choices.get(index)? {
        Choice::Projected(choice) => Some(choice.finish_reason),
        Choice::EmptyDelta { .. } => None,
    }
}

#[test]
fn chunks_split_after_a_valid_fragment_switches_to_accumulation() {
    let mut decoder = decoder();
    let first = decoder
        .push(br#"{"responseId":"r1","candidates":[{"content":{"parts":[{"text":"he"}]}}]}"#)
        .unwrap()
        .unwrap();
    assert_eq!(first.id, "r1");
    assert_eq!(content(&first), Some("he"));
    assert_eq!(first.created, NOW);
    assert!(!first.is_finished);
    let partial = decoder
        .push(br#"{"responseId":"r1","candidates":[{"content":{"parts":[{"text":"llo"#)
        .unwrap();
    assert!(partial.is_none());
    let last = decoder
        .push(br#""}]}}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":2,"totalTokenCount":3}}"#)
        .unwrap()
        .unwrap();
    assert_eq!(content(&last), Some("llo"));
    assert_eq!(last.usage.as_ref().unwrap().total_tokens, 3);
    assert!(decoder.finish().unwrap().is_none());
}

#[test]
fn chunks_concatenated_objects_drain_one_value_per_call() {
    let mut decoder = decoder();
    let opening = decoder
        .push(br#"{"responseId":"r0","candidates":[{"content":{"parts":[{"text":"one"}]}}]"#)
        .unwrap();
    assert!(opening.is_none());
    let first = decoder
        .push(br#"}{"responseId":"r1","candidates":[{"content":{"parts":[{"text":"two"}]}}]}"#)
        .unwrap()
        .unwrap();
    assert_eq!(first.id, "r0");
    assert_eq!(content(&first), Some("one"));
    let drained = decoder.finish().unwrap().unwrap();
    assert_eq!(drained.id, "r1");
    assert_eq!(content(&drained), Some("two"));
    assert!(decoder.finish().unwrap().is_none());
}

#[test]
fn chunks_complete_object_before_a_truncated_tail_is_drained() {
    let mut decoder = decoder();
    let waiting = decoder
        .push(br#"{"responseId":"r0","candidates":[{"content":{"parts":[{"text":"whole"}]}}]}{"responseId":"r1","candidates":[{"content":{"parts":[{"te"#)
        .unwrap();
    assert!(waiting.is_none());
    let drained = decoder.finish().unwrap().unwrap();
    assert_eq!(drained.id, "r0");
    assert_eq!(content(&drained), Some("whole"));
    assert!(decoder.finish().unwrap().is_none());
}

#[test]
fn chunks_metadata_only_candidates_yield_empty_deltas() {
    let mut decoder = decoder();
    let chunk = decoder
        .push(br#"{"responseId":"m1","candidates":[{"groundingMetadata":{"webSearchQueries":["q"]}},{"safetyRatings":[{"category":"X"}]}]}"#)
        .unwrap()
        .unwrap();
    assert_eq!(chunk.choices.len(), 2);
    let Choice::EmptyDelta { index: first } = &chunk.choices[0] else {
        panic!("a candidate without content or finishReason is skipped by the projection");
    };
    let Choice::EmptyDelta { index: second } = &chunk.choices[1] else {
        panic!("a candidate without content or finishReason is skipped by the projection");
    };
    assert_eq!(*first, 0);
    assert_eq!(*second, 0);
    assert!(chunk.metadata.grounding_metadata.is_empty());
    assert!(chunk.metadata.safety_ratings.is_empty());
    assert!(chunk.usage.is_none());
}

#[test]
fn chunks_tool_calls_continue_and_a_later_stop_becomes_tool_calls() {
    let mut decoder = decoder();
    let first = decoder
        .push(br#"{"responseId":"r1","candidates":[{"content":{"parts":[{"functionCall":{"id":"call_1","name":"lookup","args":{"city":"Paris"}}}]}}]}"#)
        .unwrap()
        .unwrap();
    assert_eq!(finish_reason(&first), Some("tool_calls"));
    let Choice::Projected(choice) = &first.choices[0] else {
        panic!("a function call is a projected choice");
    };
    let calls = choice.delta.tool_calls.as_ref().unwrap();
    assert_eq!(calls[0].index, 0);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].function.arguments, r#"{"city":"Paris"}"#);
    let second = decoder
        .push(br#"{"responseId":"r1","candidates":[{"content":{"parts":[{"functionCall":{"id":"call_2","name":"forecast","args":{}}}]}}]}"#)
        .unwrap()
        .unwrap();
    let Choice::Projected(choice) = &second.choices[0] else {
        panic!("a function call is a projected choice");
    };
    assert_eq!(choice.delta.tool_calls.as_ref().unwrap()[0].index, 1);
    let stop = decoder
        .push(br#"{"responseId":"r1","candidates":[{"content":{"parts":[{"text":""}]},"finishReason":"STOP"}]}"#)
        .unwrap()
        .unwrap();
    assert_eq!(finish_reason(&stop), Some("tool_calls"));
}

#[test]
fn chunks_legacy_function_mode_reports_a_function_call() {
    let mut decoder = decoder_with(&[], CallMode::LegacyFunction);
    let chunk = decoder
        .push(br#"{"responseId":"r1","candidates":[{"content":{"parts":[{"functionCall":{"name":"f","args":{}}}]},"finishReason":"STOP"}]}"#)
        .unwrap()
        .unwrap();
    assert_eq!(finish_reason(&chunk), Some("function_call"));
    let Choice::Projected(choice) = &chunk.choices[0] else {
        panic!("a function call is a projected choice");
    };
    assert!(choice.delta.tool_calls.is_none());
    let function = choice.delta.function_call.as_ref().unwrap();
    assert_eq!(function.name, "f");
    assert_eq!(function.arguments, "{}");
}

#[rstest]
#[case::length("MAX_TOKENS", "length")]
#[case::filtered("SAFETY", "content_filter")]
fn chunks_native_reasons_map_through_the_candidate(#[case] native: &str, #[case] mapped: &str) {
    let fragment = format!(r#"{{"responseId":"r1","candidates":[{{"finishReason":"{native}"}}]}}"#);
    let chunk = decoder().push(fragment.as_bytes()).unwrap().unwrap();
    assert_eq!(finish_reason(&chunk), Some(mapped));
}

#[test]
fn chunks_blocked_prompt_keeps_usage_traffic_and_tier() {
    let mut decoder = decoder_with(&[("x-gemini-service-tier", "STANDARD")], CallMode::Tools);
    let chunk = decoder
        .push(br#"{"responseId":"b1","promptFeedback":{"blockReason":"SAFETY"},"candidates":[{"content":{"parts":[{"text":"still"}]},"groundingMetadata":{"webSearchQueries":["q"]}}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2,"trafficType":"ON_DEMAND"}}"#)
        .unwrap()
        .unwrap();
    assert_eq!(chunk.choices.len(), 2);
    assert_eq!(finish_reason_at(&chunk, 0), Some("content_filter"));
    assert_eq!(finish_reason_at(&chunk, 1), Some("stop"));
    assert_eq!(chunk.service_tier.as_deref(), Some("default"));
    assert_eq!(chunk.traffic_type.as_deref(), Some("ON_DEMAND"));
    let usage = chunk.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 1);
    assert_eq!(usage.total_tokens, 2);
    assert_eq!(
        usage
            .prompt_tokens_details
            .as_ref()
            .unwrap()
            .web_search_requests,
        Some(1)
    );
    assert_eq!(chunk.metadata.grounding_metadata.len(), 1);
}

#[rstest]
#[case::standard("STANDARD", Some("default"))]
#[case::priority("Priority", Some("priority"))]
#[case::flex("flex", Some("flex"))]
#[case::empty("", None)]
fn chunks_service_tier_follows_the_response_header(
    #[case] value: &str,
    #[case] tier: Option<&str>,
) {
    let mut decoder = decoder_with(&[("x-gemini-service-tier", value)], CallMode::Tools);
    let chunk = decoder
        .push(br#"{"responseId":"r1","usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}"#)
        .unwrap()
        .unwrap();
    assert_eq!(chunk.service_tier.as_deref(), tier);
}

#[test]
fn chunks_service_tier_rides_only_the_chunks_with_usage() {
    let mut decoder = decoder_with(&[("x-gemini-service-tier", "flex")], CallMode::Tools);
    let plain = decoder.push(br#"{"responseId":"r1"}"#).unwrap().unwrap();
    assert!(plain.service_tier.is_none());
}

#[test]
fn chunks_blocked_prompt_with_skipped_candidates_adds_no_empty_delta() {
    let mut decoder = decoder();
    let chunk = decoder
        .push(br#"{"responseId":"b1","promptFeedback":{"blockReason":"SAFETY"},"candidates":[{"groundingMetadata":{"webSearchQueries":["q"]}}]}"#)
        .unwrap()
        .unwrap();
    assert_eq!(chunk.choices.len(), 1);
    assert_eq!(finish_reason(&chunk), Some("content_filter"));
}

#[rstest]
#[case::full(
    br#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","message":"quota exceeded"}}"#,
    "RESOURCE_EXHAUSTED - quota exceeded"
)]
#[case::defaults(br#"{"error":{"code":503}}"#, "UNKNOWN - Unknown error")]
#[case::null_fields(
    br#"{"error":{"status":null,"message":null}}"#,
    "UNKNOWN - Unknown error"
)]
fn chunks_embedded_errors_carry_status_and_message(#[case] fragment: &[u8], #[case] message: &str) {
    let error = decoder().push(fragment).unwrap_err();
    let StreamError::Embedded { message: found, .. } = error else {
        panic!("an embedded error is reported as an embedded stream error");
    };
    assert_eq!(found, message);
}

#[test]
fn chunks_a_non_object_error_names_the_value() {
    let error = decoder().push(br#"{"error":"boom"}"#).unwrap_err();
    let StreamError::Embedded { message, .. } = error else {
        panic!("an embedded error is reported as an embedded stream error");
    };
    assert_eq!(message, "Unexpected error format in mid-stream chunk: boom");
}

#[test]
fn chunks_without_candidates_leave_choices_and_usage_empty() {
    let mut decoder = decoder();
    let chunk = decoder.push(br#"{"responseId":"r1"}"#).unwrap().unwrap();
    assert!(chunk.choices.is_empty());
    assert!(chunk.usage.is_none());
    assert!(chunk.model.is_none());
    assert!(chunk.traffic_type.is_none());
}

#[test]
fn chunks_usage_without_candidates_is_still_counted() {
    let mut decoder = decoder();
    let chunk = decoder
        .push(br#"{"responseId":"r1","usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":1,"totalTokenCount":5}}"#)
        .unwrap()
        .unwrap();
    assert!(chunk.choices.is_empty());
    assert_eq!(chunk.usage.as_ref().unwrap().prompt_tokens, 4);
}

#[test]
fn chunks_keep_the_served_model_prefix() {
    let mut decoder = decoder();
    let chunk = decoder
        .push(br#"{"responseId":"r1","modelVersion":"gemini-2.5-pro@2025-06-17"}"#)
        .unwrap()
        .unwrap();
    assert_eq!(chunk.model.as_deref(), Some("gemini-2.5-pro"));
}

#[test]
fn chunks_draw_the_chunk_id_before_the_tool_call_id() {
    let mut next = 0_u128;
    let ids = Box::new(move || {
        next = next.saturating_add(1);
        Uuid::from_u128(next)
    });
    let mut decoder = GeminiStreamDecoder::new(options(), &HeaderMap::new(), ids);
    let chunk = decoder
        .push(
            br#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"f","args":{}}}]}}]}"#,
        )
        .unwrap()
        .unwrap();
    assert_eq!(chunk.id, "chatcmpl-00000000-0000-0000-0000-000000000001");
    let Choice::Projected(choice) = &chunk.choices[0] else {
        panic!("a function call is a projected choice");
    };
    let calls = choice.delta.tool_calls.as_ref().unwrap();
    assert!(calls[0].id.starts_with("call_"));
    assert_ne!(calls[0].id, chunk.id);
}

#[test]
fn chunks_empty_fragment_keeps_the_valid_json_mode() {
    let mut decoder = decoder();
    assert!(decoder.push(b"").unwrap().is_none());
    assert!(matches!(decoder.push(b"7"), Err(StreamError::NotAnObject)));
}

#[test]
fn chunks_whitespace_only_fragment_switches_to_accumulated_mode() {
    let mut decoder = decoder();
    assert!(decoder.push(b"   ").unwrap().is_none());
    let chunk = decoder.push(br#"7{"responseId":"r1"}"#).unwrap().unwrap();
    assert_eq!(chunk.id, "r1");
}

#[rstest]
#[case::array(br#"[{"candidates":[]}]"#)]
#[case::number(b"7")]
#[case::text(br#""just text""#)]
fn chunks_valid_mode_rejects_a_non_object_value(#[case] fragment: &[u8]) {
    let mut decoder = decoder();
    assert!(matches!(
        decoder.push(fragment),
        Err(StreamError::NotAnObject)
    ));
}

#[rstest]
#[case::space_prefix(b"data: {\"responseId\":\"r1\"}")]
#[case::bare_prefix(b"data:{\"responseId\":\"r1\"}")]
#[case::double_prefix(b"data: data: {\"responseId\":\"r1\"}")]
fn chunks_strip_the_sse_data_prefix(#[case] fragment: &[u8]) {
    let mut decoder = decoder();
    let chunk = decoder.push(fragment).unwrap().unwrap();
    assert_eq!(chunk.id, "r1");
}

#[test]
fn chunks_accumulated_mode_removes_double_newlines() {
    let mut decoder = decoder();
    assert!(decoder.push(br#"{"responseId":"#).unwrap().is_none());
    let chunk = decoder.push(b"\"r\n\n1\"}").unwrap().unwrap();
    assert_eq!(chunk.id, "r1");
}

#[rstest]
#[case::number(
    br#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","message":"quota"}}"#,
    429
)]
#[case::text_code(br#"{"error":{"code":"429"}}"#, 429)]
#[case::absent_code(br#"{"error":{"message":"m"}}"#, 500)]
#[case::null_code(br#"{"error":{"code":null}}"#, 500)]
#[case::uncoercible(br#"{"error":{"code":"many"}}"#, 500)]
#[case::boolean_code(br#"{"error":{"code":true}}"#, 1)]
#[case::empty_object(br#"{"error":{}}"#, 500)]
#[case::string_error(br#"{"error":"boom"}"#, 500)]
#[case::null_error(br#"{"error":null}"#, 500)]
fn chunks_embedded_errors_use_their_coercible_status(#[case] fragment: &[u8], #[case] status: u16) {
    let mut decoder = decoder();
    let error = decoder.push(fragment).unwrap_err();
    let StreamError::Embedded { status: found, .. } = error else {
        panic!("an embedded error is reported as an embedded stream error");
    };
    assert_eq!(found, status);
}

#[test]
fn ownership_the_retained_buffer_accepts_exactly_the_limit() {
    let mut decoder = decoder();
    let exact = "x".repeat(MAX_RETAINED_BYTES);
    assert!(decoder.push(exact.as_bytes()).unwrap().is_none());
}

#[test]
fn ownership_the_retained_buffer_counts_the_unparsed_fragments() {
    let mut decoder = decoder();
    let half = "x".repeat(MAX_RETAINED_BYTES / 2 + 1);
    assert!(decoder.push(half.as_bytes()).unwrap().is_none());
    assert!(matches!(
        decoder.push(half.as_bytes()),
        Err(StreamError::TooLarge { .. })
    ));
}

#[test]
fn ownership_the_retained_buffer_stops_at_the_limit() {
    let mut decoder = decoder();
    let oversized = "x".repeat(MAX_RETAINED_BYTES + 1);
    assert!(matches!(
        decoder.push(oversized.as_bytes()),
        Err(StreamError::TooLarge { .. })
    ));
}

#[test]
fn ownership_cursor_keeps_the_copy_cost_linear() {
    let mut decoder = decoder();
    let payload: String = (0..50)
        .map(|position| format!(r#"{{"responseId":"r{position}"}}"#))
        .collect();
    let first = decoder.push(payload.as_bytes()).unwrap().unwrap();
    assert_eq!(first.id, "r0");
    let mut drained = 0_usize;
    while let Some(chunk) = decoder.finish().unwrap() {
        drained = drained.saturating_add(1);
        assert_eq!(chunk.id, format!("r{drained}"));
    }
    assert_eq!(drained, 49);
    assert!(decoder.buffer.copied <= payload.len().saturating_mul(2));
    assert!(decoder.buffer.text.is_empty());
}

struct CountingReader {
    fragments: Vec<String>,
    closed: Arc<AtomicUsize>,
    fail: bool,
}

impl FragmentReader for CountingReader {
    fn next_fragment(&mut self) -> Result<Option<String>, StreamError> {
        Ok(if self.fragments.is_empty() {
            None
        } else {
            Some(self.fragments.remove(0))
        })
    }

    fn close(&mut self) -> Result<(), StreamError> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(StreamError::Close);
        }
        Ok(())
    }
}

struct CountingResponse {
    closed: Arc<AtomicUsize>,
    fail: bool,
}

impl ResponseClose for CountingResponse {
    fn close_response(&mut self) -> Result<(), StreamError> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(StreamError::Close);
        }
        Ok(())
    }
}

struct AsyncSource {
    fragments: Vec<String>,
    closed: Arc<AtomicUsize>,
}

impl AsyncFragmentReader for AsyncSource {
    fn next_fragment(&mut self) -> StreamFuture<'_, Result<Option<String>, StreamError>> {
        Box::pin(async move {
            Ok(if self.fragments.is_empty() {
                None
            } else {
                Some(self.fragments.remove(0))
            })
        })
    }

    fn close(&mut self) -> StreamFuture<'_, Result<(), StreamError>> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

struct AsyncResponse {
    closed: Arc<AtomicUsize>,
}

impl ResponseClose for AsyncResponse {
    fn close_response(&mut self) -> Result<(), StreamError> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct CountingFixture {
    stream: GeminiStream<CountingReader, CountingResponse>,
    reader_closed: Arc<AtomicUsize>,
    response_closed: Arc<AtomicUsize>,
}

fn counting_stream(fragments: &[&str], reader_fail: bool, response_fail: bool) -> CountingFixture {
    let reader_closed = Arc::new(AtomicUsize::new(0));
    let response_closed = Arc::new(AtomicUsize::new(0));
    let reader = CountingReader {
        fragments: fragments.iter().map(|value| (*value).to_owned()).collect(),
        closed: Arc::clone(&reader_closed),
        fail: reader_fail,
    };
    let response = CountingResponse {
        closed: Arc::clone(&response_closed),
        fail: response_fail,
    };
    CountingFixture {
        stream: GeminiStream::new(
            reader,
            Some(response),
            options(),
            &HeaderMap::new(),
            fixed_ids(),
        ),
        reader_closed,
        response_closed,
    }
}

#[test]
fn ownership_close_shuts_the_reader_and_the_response() {
    let fixture = counting_stream(&[r#"{"responseId":"r1"}"#], false, false);
    let CountingFixture {
        mut stream,
        reader_closed,
        response_closed,
    } = fixture;
    assert!(stream.next().unwrap().unwrap().is_some());
    stream.close();
    assert_eq!(reader_closed.load(Ordering::SeqCst), 1);
    assert_eq!(response_closed.load(Ordering::SeqCst), 1);
}

#[test]
fn ownership_a_failed_close_is_diagnostic_only() {
    let fixture = counting_stream(&[r#"{"responseId":"r1"}"#], true, true);
    let CountingFixture {
        mut stream,
        reader_closed,
        response_closed,
    } = fixture;
    assert!(stream.next().unwrap().unwrap().is_some());
    stream.close();
    assert_eq!(reader_closed.load(Ordering::SeqCst), 1);
    assert_eq!(response_closed.load(Ordering::SeqCst), 1);
    stream.close();
    assert_eq!(reader_closed.load(Ordering::SeqCst), 2);
    assert_eq!(response_closed.load(Ordering::SeqCst), 2);
}

#[test]
fn ownership_a_failed_close_is_logged_for_each_owner() {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let mut stream = counting_stream(&[], true, true).stream;
    with_default(subscriber, || stream.close());
    let logged = fs::read_to_string(log.path()).unwrap();
    assert!(logged.contains("owner=\"reader\""));
    assert!(logged.contains("owner=\"response\""));
}

#[test]
fn ownership_a_clean_close_logs_nothing() {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let mut stream = counting_stream(&[], false, false).stream;
    with_default(subscriber, || stream.close());
    assert!(fs::read_to_string(log.path()).unwrap().is_empty());
}

#[test]
fn ownership_the_adapter_ends_after_a_stream_error() {
    let fixture = counting_stream(&[r#"{"error":{"code":429}}"#], false, false);
    let mut stream = fixture.stream;
    assert!(matches!(
        stream.next(),
        Some(Err(StreamError::Embedded { status: 429, .. }))
    ));
    assert!(stream.next().is_none());
}

#[test]
fn ownership_the_sync_adapter_drains_the_buffer_at_eof() {
    let fixture = counting_stream(&[r#"{"responseId":"r1"}{"responseId":"r2"}"#], false, false);
    let mut stream = fixture.stream;
    let first = stream.next().unwrap().unwrap().unwrap();
    assert_eq!(first.id, "r1");
    let second = stream.next().unwrap().unwrap().unwrap();
    assert_eq!(second.id, "r2");
    assert!(stream.next().is_none());
}

#[tokio::test]
async fn ownership_the_async_adapter_yields_chunks_and_drains_at_eof() {
    let closed = Arc::new(AtomicUsize::new(0));
    let source = AsyncSource {
        fragments: vec![
            r#"{"responseId":"r1"}{"responseId":"r2"}"#.to_owned(),
            String::from(" "),
        ],
        closed: Arc::clone(&closed),
    };
    let mut stream = GeminiStream::new(
        source,
        None::<AsyncResponse>,
        options(),
        &HeaderMap::new(),
        fixed_ids(),
    );
    let first = stream.next_chunk().await.unwrap().unwrap().unwrap();
    assert_eq!(first.id, "r1");
    let second = stream.next_chunk().await.unwrap().unwrap().unwrap();
    assert_eq!(second.id, "r2");
    assert!(stream.next_chunk().await.is_none());
}

#[tokio::test]
async fn ownership_async_close_shuts_the_reader_and_the_response() {
    let reader_closed = Arc::new(AtomicUsize::new(0));
    let response_closed = Arc::new(AtomicUsize::new(0));
    let source = AsyncSource {
        fragments: vec![r#"{"responseId":"r1"}"#.to_owned()],
        closed: Arc::clone(&reader_closed),
    };
    let response = AsyncResponse {
        closed: Arc::clone(&response_closed),
    };
    let mut stream = GeminiStream::new(
        source,
        Some(response),
        options(),
        &HeaderMap::new(),
        fixed_ids(),
    );
    assert!(stream.next_chunk().await.unwrap().unwrap().is_some());
    stream.aclose().await;
    assert_eq!(reader_closed.load(Ordering::SeqCst), 1);
    assert_eq!(response_closed.load(Ordering::SeqCst), 1);
}

#[test]
fn ownership_a_utf8_fragment_decodes_before_parsing() {
    let mut decoder = decoder();
    let chunk = decoder
        .push(r#"{"responseId":"héllo"}"#.as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(chunk.id, "héllo");
}

#[test]
fn ownership_invalid_utf8_is_an_error() {
    let mut decoder = decoder();
    assert!(matches!(
        decoder.push(&[0x7b, 0x22, 0xff, 0x22, 0x7d]),
        Err(StreamError::Utf8 { .. })
    ));
}

#[rstest]
fn replays_the_vertex_stream_corpus(#[files("fuzz/corpus/vertex_stream/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let mut decoder = decoder();
    for fragment in bytes.split(|byte| *byte == b'\n') {
        drop(decoder.push(fragment));
    }
    while let Ok(Some(_chunk)) = decoder.finish() {}
}
