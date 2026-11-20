#![cfg(test)]

use std::fs;
use std::io;
use std::path::PathBuf;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use betterlite_anthropic::api::messages_errors::frame::anthropic_error_sse_frame;
use betterlite_models::decisions::Json;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::answer::{HeaderRefresh, HeaderRefreshError, first_chunk_answer};
use super::frame_error::{frame_error_code, frame_error_object};
use crate::first_byte_sse::{ProducedResponse, StreamedResponse};
use crate::shared_stream::call::StreamAbort;
use crate::shared_stream::generator::DEFAULT_LIMITS;
use crate::shared_stream::sse::Emitted;

type Items = Vec<Result<Emitted, StreamAbort>>;

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

struct FixedHeaders(HeaderMap);

#[async_trait]
impl HeaderRefresh for FixedHeaders {
    async fn refresh(&self) -> Result<HeaderMap, HeaderRefreshError> {
        Ok(self.0.clone())
    }
}

struct RefusedRefresh;

#[async_trait]
impl HeaderRefresh for RefusedRefresh {
    async fn refresh(&self) -> Result<HeaderMap, HeaderRefreshError> {
        Err(HeaderRefreshError {
            message: String::from("refresh failed"),
        })
    }
}

fn frame(text: &str) -> Emitted {
    Emitted::Frame(Bytes::from(text.to_owned()))
}

fn anthropic_error(status: u16, message: &str) -> Emitted {
    let upstream =
        anthropic_error_sse_frame(status, message, DEFAULT_LIMITS).expect("an error frame");
    Emitted::AnthropicError(Box::new(upstream))
}

fn failure(status: u16, detail: &str) -> Result<Emitted, StreamAbort> {
    Err(StreamAbort {
        status,
        detail: detail.to_owned(),
    })
}

fn scripted(items: Items) -> BoxStream<'static, Result<Emitted, StreamAbort>> {
    stream::iter(items).boxed()
}

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for &(name, value) in pairs {
        map.insert(
            HeaderName::from_static(name),
            HeaderValue::from_str(value).expect("a header value"),
        );
    }
    map
}

async fn answer(
    items: Items,
    given: HeaderMap,
    refresh: Option<Arc<dyn HeaderRefresh>>,
    call_id_in_body: bool,
) -> ProducedResponse {
    first_chunk_answer(scripted(items), given, refresh, call_id_in_body).await
}

fn streaming(produced: ProducedResponse) -> StreamedResponse {
    match produced {
        ProducedResponse::Streaming(streamed) => streamed,
        ProducedResponse::Plain(response) => {
            panic!("expected a streaming answer, got {}", response.status())
        }
    }
}

fn plain(produced: ProducedResponse) -> Response {
    match produced {
        ProducedResponse::Plain(response) => response,
        ProducedResponse::Streaming(_) => panic!("expected a plain JSON answer"),
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.to_str().expect("a text header").to_owned())
}

async fn body(response: Response) -> Bytes {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a response body")
}

async fn stream_body(streamed: StreamedResponse) -> Bytes {
    to_bytes(Body::from_stream(streamed.body), usize::MAX)
        .await
        .expect("a streamed body")
}

fn json_bytes<T>(value: &T) -> Bytes
where
    T: serde::Serialize,
{
    Bytes::from(serde_json::to_vec(value).expect("serialized JSON"))
}

fn serialized_object(frame: &[u8]) -> Bytes {
    json_bytes(&Json::Object(frame_error_object(frame)))
}

#[rstest]
#[case::a_plain_code("data: {\"error\": {\"code\": 404}}", Some(404))]
#[case::the_lowest_code("data: {\"error\": {\"code\": 100}}", Some(100))]
#[case::the_highest_code("data: {\"error\": {\"code\": 599}}", Some(599))]
#[case::one_below_the_range("data: {\"error\": {\"code\": 99}}", None)]
#[case::one_above_the_range("data: {\"error\": {\"code\": 600}}", None)]
#[case::a_digit_string("data: {\"error\": {\"code\": \"404\"}}", Some(404))]
#[case::a_padded_string("data: {\"error\": {\"code\": \" 404 \"}}", Some(404))]
#[case::a_signed_string("data: {\"error\": {\"code\": \"+404\"}}", Some(404))]
#[case::an_underscored_string("data: {\"error\": {\"code\": \"4_04\"}}", Some(404))]
#[case::a_negative_string("data: {\"error\": {\"code\": \"-404\"}}", None)]
#[case::a_non_numeric_string("data: {\"error\": {\"code\": \"nope\"}}", None)]
#[case::a_float("data: {\"error\": {\"code\": 404.0}}", None)]
#[case::a_bool("data: {\"error\": {\"code\": true}}", None)]
#[case::a_null_code("data: {\"error\": {\"code\": null}}", None)]
#[case::a_missing_code("data: {\"error\": {}}", None)]
#[case::an_error_that_is_not_an_object("data: {\"error\": \"nope\"}", None)]
#[case::a_payload_that_is_not_an_object("data: [1, 2]", None)]
#[case::no_data_prefix("{\"error\": {\"code\": 404}}", None)]
#[case::a_prefix_without_the_space("data:{\"error\": {\"code\": 404}}", None)]
#[case::an_empty_payload("data: ", None)]
#[case::a_whitespace_payload("data:   ", None)]
#[case::a_done_marker("data: [DONE]", None)]
#[case::bad_json("data: {not json", None)]
#[case::a_trailing_blank_line("data: {\"error\": {\"code\": 404}}\n\n", Some(404))]
fn frame_error_code_reads_the_first_chunk(#[case] frame: &str, #[case] code: Option<u16>) {
    assert_eq!(frame_error_code(frame.as_bytes()), code);
}

#[test]
fn frame_error_code_refuses_bytes_that_are_not_utf8() {
    assert_eq!(
        frame_error_code(b"data: {\"error\": {\"code\": 404}}\xff"),
        None
    );
}

#[test]
fn a_non_numeric_code_string_warns_twice() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    assert_eq!(
        frame_error_code(b"data: {\"error\": {\"code\": \"nope\"}}"),
        None
    );
    let contents = logs.contents();
    assert!(contents.contains("Error code is a string but not a valid integer: nope"));
    assert!(contents.contains("Error has invalid or non-convertible code: nope"));
}

#[test]
fn an_out_of_range_code_warns_once_with_its_number() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    assert_eq!(
        frame_error_code(b"data: {\"error\": {\"code\": 700}}"),
        None
    );
    let contents = logs.contents();
    assert!(!contents.contains("not a valid integer"));
    assert!(contents.contains("Error has invalid or non-convertible code: 700"));
}

#[test]
fn a_code_string_that_overflows_an_integer_warns_twice() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    assert_eq!(
        frame_error_code(b"data: {\"error\": {\"code\": \"99999999999999999999999999\"}}"),
        None
    );
    let contents = logs.contents();
    assert!(contents.contains("not a valid integer: 99999999999999999999999999"));
    assert!(
        contents.contains("Error has invalid or non-convertible code: 99999999999999999999999999")
    );
}

#[rstest]
#[case::a_bool("true", "true")]
#[case::a_float("404.5", "404.5")]
#[case::a_list("[404]", "[404]")]
#[case::an_object("{\"a\": 1}", "{\"a\":1}")]
fn an_odd_code_warns_with_its_json(#[case] raw: &str, #[case] rendered: &str) {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let chunk = format!("data: {{\"error\": {{\"code\": {raw}}}}}");
    assert_eq!(frame_error_code(chunk.as_bytes()), None);
    assert!(logs.contents().contains(&format!(
        "Error has invalid or non-convertible code: {rendered}"
    )));
}

#[test]
fn a_usable_code_warns_about_nothing() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    assert_eq!(
        frame_error_code(b"data: {\"error\": {\"code\": \"404\"}}"),
        Some(404)
    );
    assert!(logs.contents().is_empty());
}

#[test]
fn a_null_or_missing_code_warns_about_nothing() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    assert_eq!(
        frame_error_code(b"data: {\"error\": {\"code\": null}}"),
        None
    );
    assert_eq!(
        frame_error_code(b"data: {\"error\": {\"message\": \"x\"}}"),
        None
    );
    assert!(logs.contents().is_empty());
}

#[test]
fn a_code_string_of_non_ascii_digits_warns_twice() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let digits: String = [0x0664_u32, 0x0660_u32, 0x0664_u32]
        .into_iter()
        .filter_map(char::from_u32)
        .collect();
    let chunk = format!("data: {{\"error\": {{\"code\": \"{digits}\"}}}}");
    assert_eq!(frame_error_code(chunk.as_bytes()), None);
    let contents = logs.contents();
    assert!(contents.contains(&format!("not a valid integer: {digits}")));
    assert!(contents.contains(&format!(
        "Error has invalid or non-convertible code: {digits}"
    )));
}

#[test]
fn frame_error_object_defaults_with_litellms_key_order() {
    assert_eq!(
        serialized_object(b"data: {\"error\": \"nope\"}"),
        Bytes::from_static(
            br#"{"message":"Unknown error","type":"internal_server_error","param":null,"code":"500"}"#
        )
    );
}

#[test]
fn frame_error_object_passes_the_error_through_unchanged() {
    assert_eq!(
        serialized_object(br#"data: {"error": {"message": "boom", "code": "500", "type": "x"}}"#),
        Bytes::from_static(br#"{"message":"boom","code":"500","type":"x"}"#)
    );
}

#[rstest]
#[case::no_prefix("{\"error\": {}}")]
#[case::bad_json("data: {oops")]
#[case::a_done_marker("data: [DONE]")]
#[case::an_empty_payload("data: ")]
#[case::an_array_payload("data: [1]")]
#[case::no_error_key("data: {\"other\": 1}")]
#[case::an_error_that_is_a_string("data: {\"error\": \"boom\"}")]
fn frame_error_object_falls_back_to_the_default(#[case] frame: &str) {
    assert_eq!(
        serialized_object(frame.as_bytes()),
        Bytes::from_static(
            br#"{"message":"Unknown error","type":"internal_server_error","param":null,"code":"500"}"#
        )
    );
}

#[test]
fn frame_error_object_refuses_bytes_that_are_not_utf8() {
    assert_eq!(
        serialized_object(b"data: \xff"),
        Bytes::from_static(
            br#"{"message":"Unknown error","type":"internal_server_error","param":null,"code":"500"}"#
        )
    );
}

#[tokio::test]
async fn an_anthropic_error_first_frame_answers_plain_json_with_its_status() {
    let produced = answer(
        vec![
            Ok(anthropic_error(429, "slow down")),
            Ok(frame("data: {\"late\": true}")),
        ],
        headers(&[("x-litellm-call-id", "call-1"), ("x-custom", "kept")]),
        None,
        false,
    )
    .await;
    let response = plain(produced);
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        header(response.headers(), CONTENT_TYPE.as_str()).as_deref(),
        Some("application/json")
    );
    assert_eq!(
        header(response.headers(), "x-custom").as_deref(),
        Some("kept")
    );
    assert_eq!(header(response.headers(), "cache-control"), None);
    assert_eq!(header(response.headers(), "x-accel-buffering"), None);
    let expected = anthropic_error_sse_frame(429, "slow down", DEFAULT_LIMITS)
        .expect("an error frame")
        .json_body(None)
        .expect("a JSON error body");
    assert_eq!(body(response).await, json_bytes(&expected));
}

#[rstest]
#[case::not_included(false)]
#[case::included(true)]
#[tokio::test]
async fn an_anthropic_error_body_carries_the_call_id_only_when_enabled(
    #[case] call_id_in_body: bool,
) {
    let produced = answer(
        vec![Ok(anthropic_error(400, "bad request"))],
        headers(&[("x-litellm-call-id", "call-1")]),
        None,
        call_id_in_body,
    )
    .await;
    let text = String::from_utf8(body(plain(produced)).await.to_vec()).expect("a JSON body");
    assert_eq!(
        text.contains("\"litellm_call_id\":\"call-1\""),
        call_id_in_body
    );
}

#[tokio::test]
async fn an_anthropic_error_reads_the_call_id_from_the_refreshed_headers() {
    let refresh = FixedHeaders(headers(&[("x-litellm-call-id", "call-2")]));
    let produced = answer(
        vec![Ok(anthropic_error(400, "bad request"))],
        headers(&[("x-litellm-call-id", "call-1")]),
        Some(Arc::new(refresh)),
        true,
    )
    .await;
    let response = plain(produced);
    assert_eq!(
        header(response.headers(), "x-litellm-call-id").as_deref(),
        Some("call-2")
    );
    assert_ne!(
        header(response.headers(), "x-litellm-call-id").as_deref(),
        Some("call-1")
    );
    let text = String::from_utf8(body(response).await.to_vec()).expect("a JSON body");
    assert!(text.contains("\"litellm_call_id\":\"call-2\""));
    assert!(!text.contains("call-1"));
}

#[tokio::test]
async fn a_first_frame_with_a_code_answers_plain_json_with_that_status() {
    let produced = answer(
        vec![
            Ok(frame(
                "data: {\"error\": {\"message\": \"boom\", \"code\": 503}}",
            )),
            Ok(frame("data: never read")),
        ],
        headers(&[("x-custom", "kept"), ("cache-control", "max-age=60")]),
        None,
        false,
    )
    .await;
    let response = plain(produced);
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        header(response.headers(), CONTENT_TYPE.as_str()).as_deref(),
        Some("application/json")
    );
    assert_eq!(
        header(response.headers(), "x-custom").as_deref(),
        Some("kept")
    );
    assert_eq!(
        header(response.headers(), "cache-control").as_deref(),
        Some("max-age=60")
    );
    assert_eq!(header(response.headers(), "x-accel-buffering"), None);
    assert_eq!(
        body(response).await,
        Bytes::from_static(br#"{"error":{"message":"boom","code":503}}"#)
    );
}

#[tokio::test]
async fn a_first_frame_error_body_carries_the_refreshed_call_id_last() {
    let refresh = FixedHeaders(headers(&[("x-litellm-call-id", "call-2")]));
    let produced = answer(
        vec![Ok(frame("data: {\"error\": {\"code\": 400}}"))],
        headers(&[("x-litellm-call-id", "call-1")]),
        Some(Arc::new(refresh)),
        true,
    )
    .await;
    let response = plain(produced);
    assert_eq!(
        header(response.headers(), "x-litellm-call-id").as_deref(),
        Some("call-2")
    );
    assert_eq!(
        body(response).await,
        Bytes::from_static(br#"{"error":{"code":400,"litellm_call_id":"call-2"}}"#)
    );
}

#[tokio::test]
async fn a_first_frame_error_body_leaves_the_call_id_out_when_disabled() {
    let produced = answer(
        vec![Ok(frame("data: {\"error\": {\"code\": 400}}"))],
        headers(&[("x-litellm-call-id", "call-1")]),
        None,
        false,
    )
    .await;
    assert_eq!(
        body(plain(produced)).await,
        Bytes::from_static(br#"{"error":{"code":400}}"#)
    );
}

#[tokio::test]
async fn a_first_frame_error_body_replaces_an_upstream_call_id_in_place() {
    let produced = answer(
        vec![Ok(frame(
            "data: {\"error\": {\"litellm_call_id\": \"upstream\", \"code\": 400}}",
        ))],
        headers(&[("x-litellm-call-id", "call-9")]),
        None,
        true,
    )
    .await;
    assert_eq!(
        body(plain(produced)).await,
        Bytes::from_static(br#"{"error":{"litellm_call_id":"call-9","code":400}}"#)
    );
}

#[tokio::test]
async fn a_first_frame_with_a_code_logs_the_debug_line() {
    let (logs, _guard) = capture_logs(tracing::Level::DEBUG);
    let produced = answer(
        vec![Ok(frame("data: {\"error\": {\"code\": 404}}"))],
        HeaderMap::new(),
        None,
        false,
    )
    .await;
    drop(plain(produced));
    assert!(logs.contents().contains(
        "Error detected in first stream chunk. Returning JSON error response with status code: 404"
    ));
}

#[tokio::test]
async fn a_normal_first_chunk_streams_every_item_in_order() {
    let produced = answer(
        vec![
            Ok(frame("data: first\n\n")),
            Ok(anthropic_error(500, "late failure")),
            Ok(frame("data: last\n\n")),
        ],
        headers(&[("x-custom", "kept")]),
        None,
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(streamed.status, StatusCode::OK);
    assert_eq!(
        header(&streamed.headers, CONTENT_TYPE.as_str()).as_deref(),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(
        header(&streamed.headers, "cache-control").as_deref(),
        Some("no-cache")
    );
    assert_eq!(
        header(&streamed.headers, "x-accel-buffering").as_deref(),
        Some("no")
    );
    assert_eq!(
        header(&streamed.headers, "x-custom").as_deref(),
        Some("kept")
    );
    let late = anthropic_error_sse_frame(500, "late failure", DEFAULT_LIMITS)
        .expect("an error frame")
        .body;
    let expected = format!("data: first\n\n{late}data: last\n\n");
    assert_eq!(stream_body(streamed).await, Bytes::from(expected));
}

#[tokio::test]
async fn a_first_frame_that_is_not_utf8_streams_its_bytes() {
    let produced = answer(
        vec![Ok(Emitted::Frame(Bytes::from_static(
            b"data: {\"error\": {\"code\": 404}}\xff",
        )))],
        HeaderMap::new(),
        None,
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(streamed.status, StatusCode::OK);
    assert_eq!(
        stream_body(streamed).await,
        Bytes::from_static(b"data: {\"error\": {\"code\": 404}}\xff")
    );
}

#[tokio::test]
async fn the_sse_headers_replace_the_callers_own_values() {
    let produced = answer(
        vec![Ok(frame("data: x"))],
        headers(&[
            ("cache-control", "max-age=60"),
            ("x-accel-buffering", "yes"),
        ]),
        None,
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(
        header(&streamed.headers, "cache-control").as_deref(),
        Some("no-cache")
    );
    assert_eq!(
        header(&streamed.headers, "x-accel-buffering").as_deref(),
        Some("no")
    );
}

#[tokio::test]
async fn a_refresher_replaces_the_headers_before_the_sse_ones_are_added() {
    let refresh = FixedHeaders(headers(&[("x-custom", "refreshed")]));
    let produced = answer(
        vec![Ok(frame("data: x"))],
        headers(&[("x-custom", "original")]),
        Some(Arc::new(refresh)),
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(
        header(&streamed.headers, "x-custom").as_deref(),
        Some("refreshed")
    );
    assert_eq!(
        header(&streamed.headers, "cache-control").as_deref(),
        Some("no-cache")
    );
}

#[tokio::test]
async fn a_failing_refresher_keeps_the_original_headers_and_logs() {
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let produced = answer(
        vec![Ok(frame("data: x"))],
        headers(&[("x-custom", "original")]),
        Some(Arc::new(RefusedRefresh)),
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(
        header(&streamed.headers, "x-custom").as_deref(),
        Some("original")
    );
    assert!(
        logs.contents()
            .contains("Error refreshing streaming response headers: refresh failed")
    );
}

#[tokio::test]
async fn an_empty_stream_answers_an_empty_200_sse_body() {
    let refresh = FixedHeaders(headers(&[("x-custom", "refreshed")]));
    let produced = answer(
        vec![],
        headers(&[("x-custom", "kept")]),
        Some(Arc::new(refresh)),
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(streamed.status, StatusCode::OK);
    assert_eq!(
        header(&streamed.headers, CONTENT_TYPE.as_str()).as_deref(),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(
        header(&streamed.headers, "x-custom").as_deref(),
        Some("refreshed")
    );
    assert_eq!(
        header(&streamed.headers, "cache-control").as_deref(),
        Some("no-cache")
    );
    assert!(stream_body(streamed).await.is_empty());
}

#[tokio::test]
async fn a_first_item_failure_answers_the_two_sse_error_frames_with_its_status() {
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let refresh = FixedHeaders(headers(&[("x-custom", "refreshed")]));
    let produced = answer(
        vec![failure(503, "upstream unavailable")],
        headers(&[("x-custom", "kept")]),
        Some(Arc::new(refresh)),
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(streamed.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        header(&streamed.headers, CONTENT_TYPE.as_str()).as_deref(),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(
        header(&streamed.headers, "x-custom").as_deref(),
        Some("refreshed")
    );
    assert_eq!(
        header(&streamed.headers, "cache-control").as_deref(),
        Some("no-cache")
    );
    let text = String::from_utf8(stream_body(streamed).await.to_vec()).expect("an SSE body");
    assert!(text.starts_with("data: {\"error\":{"));
    assert!(text.contains("\"message\":\"upstream unavailable\""));
    assert!(text.contains("\"code\":\"503\""));
    assert!(text.ends_with("data: [DONE]\n\n"));
    assert!(
        logs.contents()
            .contains("Error consuming first chunk from generator: upstream unavailable")
    );
}

#[tokio::test]
async fn a_later_failure_ends_the_body_after_the_bytes_already_sent() {
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let produced = answer(
        vec![
            Ok(frame("data: first")),
            Ok(frame("data: second")),
            failure(502, "broken later"),
            Ok(frame("data: never")),
        ],
        HeaderMap::new(),
        None,
        false,
    )
    .await;
    let streamed = streaming(produced);
    assert_eq!(
        stream_body(streamed).await,
        Bytes::from_static(b"data: firstdata: second")
    );
    let contents = logs.contents();
    assert!(contents.contains("broken later"));
    assert!(contents.contains("502"));
}

#[rstest]
fn replays_the_first_chunk_error_corpus(#[files("fuzz/corpus/first_chunk_error/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus seed");
    let code = frame_error_code(&bytes);
    assert!(code.is_none_or(|value| (100_u16..=599).contains(&value)));
    let object = Json::Object(frame_error_object(&bytes));
    let written = serde_json::to_string(&object).expect("the error object serializes");
    assert_eq!(
        serde_json::from_str::<Json>(&written).expect("the error object parses back"),
        object
    );
}
