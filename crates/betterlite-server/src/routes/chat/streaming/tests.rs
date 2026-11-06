#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_http::error::HttpError;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::stream::SseParser;
use futures_util::{StreamExt as _, stream};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use tower::ServiceExt as _;
use uuid::Uuid;

use super::{RelayState, next_sse};
use crate::routes::chat::chat_routes_with_call_id;
use crate::server::chat_setup::chat_executor;

fn executor_for(source: &str) -> ChatExecutor {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(3)).expect("executor")
}

async fn mock_call(source: &str, body: &str) -> (StatusCode, Vec<u8>) {
    mock_call_at("/v1/chat/completions", source, body).await
}

async fn mock_call_at(path: &str, source: &str, body: &str) -> (StatusCode, Vec<u8>) {
    routed_at(path, executor_for(source), body).await
}

async fn routed(executor: ChatExecutor, body: &str) -> (StatusCode, Vec<u8>) {
    routed_at("/v1/chat/completions", executor, body).await
}

async fn routed_at(path: &str, executor: ChatExecutor, body: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let response = chat_routes_with_call_id(Arc::new(executor), false)
        .expect("router")
        .oneshot(request)
        .await
        .expect("response");
    let status = response.status();
    let collected = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    (status, collected.to_vec())
}

fn fixed_clock() -> u64 {
    1_700_000_000
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

#[derive(serde::Deserialize)]
struct Frame {
    choices: Vec<FrameChoice>,
}

#[derive(serde::Deserialize)]
struct FrameChoice {
    index: u32,
    delta: FrameDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct FrameDelta {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[tokio::test]
async fn mock_stream_frames_use_the_injected_id_and_clock() {
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: abcdefg, timeout: 0.1}}\n";
    let mut executor = executor_for(source);
    executor.clock = fixed_clock;
    executor.next_id = fixed_id;
    let (status, body) = routed(executor, r#"{"model":"g","messages":[],"stream":true}"#).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("utf-8 SSE");
    let first = text
        .split("\n\n")
        .next()
        .expect("first frame")
        .trim_start_matches("data: ");
    let frame: FrameHeader = serde_json::from_str(first).expect("frame");
    assert_eq!(frame.id, format!("chatcmpl-{}", fixed_id()));
    assert_eq!(frame.created, fixed_clock());
}

#[derive(serde::Deserialize)]
struct FrameHeader {
    id: String,
    created: u64,
    model: String,
}

#[tokio::test]
async fn a_mock_stream_answers_with_the_path_model() {
    let source = "model_list:\n  - {model_name: text, litellm_params: {model: openai/gpt-4o, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: hi}}\n";
    let (status, body) = mock_call_at(
        "/openai/deployments/text/chat/completions",
        source,
        r#"{"messages":[],"stream":true}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("utf-8 SSE");
    let first = text
        .split("\n\n")
        .next()
        .expect("first frame")
        .trim_start_matches("data: ");
    let frame: FrameHeader = serde_json::from_str(first).expect("frame");
    assert_eq!(frame.model, "text");
}

#[derive(serde::Deserialize)]
struct UsageFrame {
    choices: Vec<FrameChoice>,
    usage: UsageCounts,
}

#[derive(serde::Deserialize)]
struct UsageCounts {
    #[serde(rename = "prompt_tokens")]
    prompt: u64,
    #[serde(rename = "completion_tokens")]
    completion: u64,
    #[serde(rename = "total_tokens")]
    total: u64,
}

#[tokio::test]
async fn an_included_usage_chunk_closes_the_mock_stream() {
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: abcdefg, timeout: 0.1}}\n";
    let (status, body) = mock_call(
        source,
        r#"{"model":"g","messages":[],"stream":true,"stream_options":{"include_usage":true}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("utf-8 SSE");
    let frames: Vec<&str> = text
        .split("\n\n")
        .filter(|frame| !frame.is_empty())
        .collect();
    // three content chunks, the finish chunk, the usage chunk, then [DONE]
    assert_eq!(frames.len(), 6, "{text}");
    assert_eq!(frames[5], "data: [DONE]");
    let usage: UsageFrame =
        serde_json::from_str(frames[4].trim_start_matches("data: ")).expect("usage frame");
    let choice = usage.choices.first().expect("choice");
    assert_eq!(choice.index, 0);
    assert_eq!(choice.delta.role, None);
    assert_eq!(choice.delta.content, None);
    assert_eq!(choice.finish_reason, None);
    assert_eq!(usage.usage.prompt, 10);
    assert_eq!(usage.usage.completion, 20);
    assert_eq!(usage.usage.total, 30);
}

#[tokio::test]
async fn mock_stream_frames_follow_the_litellm_client_shape() {
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: abcdefg, timeout: 0.1}}\n";
    let (status, body) = mock_call(source, r#"{"model":"g","messages":[],"stream":true}"#).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("utf-8 SSE");
    let frames: Vec<&str> = text
        .split("\n\n")
        .filter(|frame| !frame.is_empty())
        .collect();
    assert_eq!(frames.len(), 5, "{text}");
    let deltas: Vec<Frame> = frames
        .iter()
        .take(3)
        .map(|frame| serde_json::from_str(frame.trim_start_matches("data: ")).expect("frame"))
        .collect();
    assert_eq!(
        deltas[0].choices[0].delta.role.as_deref(),
        Some("assistant")
    );
    assert_eq!(deltas[0].choices[0].delta.content.as_deref(), Some("abc"));
    assert_eq!(deltas[1].choices[0].delta.role, None);
    assert_eq!(deltas[1].choices[0].delta.content.as_deref(), Some("def"));
    assert_eq!(deltas[2].choices[0].delta.content.as_deref(), Some("g"));
    let finish: Frame =
        serde_json::from_str(frames[3].trim_start_matches("data: ")).expect("finish");
    assert_eq!(finish.choices[0].index, 0);
    assert_eq!(finish.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(frames[4], "data: [DONE]");
}

#[derive(serde::Deserialize)]
struct ErrorReply {
    error: ErrorFields,
}

#[derive(serde::Deserialize)]
struct ErrorFields {
    #[serde(rename = "type")]
    error_type: String,
    code: String,
}

fn sentinel_source(sentinel: &str) -> String {
    [
        "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: '",
        sentinel,
        "', timeout: 0.1}}\n",
    ]
    .concat()
}

#[rstest]
#[case::streaming_error("Exception: mock_streaming_error", 529, "internal_server_error")]
#[case::content_filter(
    "Exception: content_filter_policy: blocked",
    400,
    "invalid_request_error"
)]
#[case::context_window("litellm.ContextWindowExceededError", 400, "invalid_request_error")]
#[case::internal_server("litellm.InternalServerError", 500, "internal_server_error")]
#[tokio::test]
async fn mock_calls_behavior_tests_behavior(
    #[case] sentinel: &str,
    #[case] status: u16,
    #[case] error_type: &str,
) {
    let (observed, body) = mock_call(
        &sentinel_source(sentinel),
        r#"{"model":"g","messages":[],"stream":true}"#,
    )
    .await;
    assert_eq!(observed.as_u16(), status);
    let reply: ErrorReply = serde_json::from_slice(&body).expect("error body");
    assert_eq!(reply.error.error_type, error_type);
    assert_eq!(reply.error.code, status.to_string());
}

#[rstest]
#[case::transport(Err(HttpError::Timeout))]
#[case::invalid_json(Ok(b"data: invalid\n\n".to_vec()))]
#[case::invalid_utf8(Ok(b"data: \xff\n\n".to_vec()))]
#[case::oversized(Ok(vec![b'x'; 0x0010_0001]))]
#[tokio::test]
async fn stream_error_ends_relay(#[case] input: Result<Vec<u8>, HttpError>) {
    let state = RelayState {
        upstream: stream::iter(vec![input, Ok(b"data: [DONE]\n\n".to_vec())]).boxed(),
        parser: SseParser::default(),
        pending: VecDeque::new(),
        finished: false,
        model: String::from("group"),
        include_usage: false,
    };
    let (event, remaining) = next_sse(state).await.unwrap();
    event.unwrap_err();
    assert!(next_sse(remaining).await.is_none());
}
