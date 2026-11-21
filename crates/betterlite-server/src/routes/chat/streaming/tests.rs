#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::str::from_utf8;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::operation::chat::{ChatFailure, ChatResult, TextCallMode};
use betterlite_http::error::HttpError;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_execution::{
    CompletionAnswer, CompletionCallAnswer, CompletionStreamAnswer,
};
use betterlite_openai::text_completion_response::CompletionResponse;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use futures_util::{StreamExt as _, stream};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use tower::ServiceExt as _;
use uuid::Uuid;

use super::answered_result;
use super::completion_failure;
use super::relay::{RelaySource, TextSource};
use crate::routes::chat::streaming::ChatCalls;
use crate::routes::chat::{CallContext, ErrorLayer, chat_routes_with_call_id, dispatch_failure};
use crate::routes::completions::streaming::pump::{Pump, next_frame};
use crate::server::chat_setup::chat_executor;

fn config_for(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config")
}

fn executor_for(config: &Config) -> ChatExecutor {
    chat_executor(config, &BTreeMap::new(), StdRng::seed_from_u64(3)).expect("executor")
}

async fn mock_call(source: &str, body: &str) -> (StatusCode, Vec<u8>) {
    mock_call_at("/v1/chat/completions", source, body).await
}

async fn mock_call_at(path: &str, source: &str, body: &str) -> (StatusCode, Vec<u8>) {
    let config = config_for(source);
    routed_at(path, &config, executor_for(&config), body).await
}

async fn routed(config: &Config, executor: ChatExecutor, body: &str) -> (StatusCode, Vec<u8>) {
    routed_at("/v1/chat/completions", config, executor, body).await
}

async fn routed_at(
    path: &str,
    config: &Config,
    executor: ChatExecutor,
    body: &str,
) -> (StatusCode, Vec<u8>) {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let calls = ChatCalls::new(config, Arc::new(executor)).expect("calls");
    let response = chat_routes_with_call_id(Arc::new(calls), false)
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
    let config = config_for(source);
    let mut executor = executor_for(&config);
    executor.clock = fixed_clock;
    executor.next_id = fixed_id;
    let (status, body) = routed(
        &config,
        executor,
        r#"{"model":"g","messages":[],"stream":true}"#,
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

#[tokio::test]
async fn an_object_mock_cannot_be_streamed() {
    let source = "model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/up\n      api_base: 'http://127.0.0.1:1'\n      api_key: k\n      mock_response:\n        choices:\n          - message: {content: hi}\n";
    let (status, body) = mock_call(source, r#"{"model":"g","messages":[],"stream":true}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let reply: ErrorReply = serde_json::from_slice(&body).expect("error body");
    assert_eq!(reply.error.error_type, "invalid_request_error");
    assert_eq!(reply.error.code, "400");
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
    litellm_call_id: Option<String>,
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
    let source = RelaySource {
        upstream: stream::iter(vec![input, Ok(b"data: [DONE]\n\n".to_vec())]).boxed(),
        parser: SseParser::default(),
        model: String::from("group"),
        include_usage: false,
    };
    let (event, remaining) = next_frame::<RelaySource, io::Error>(Pump::new(source))
        .await
        .unwrap();
    let _failure = event.unwrap_err();
    assert!(
        next_frame::<RelaySource, io::Error>(remaining)
            .await
            .is_none()
    );
}

#[rstest]
#[case::mismatched_operation(ChatFailure::Mismatched)]
#[case::internal(ChatFailure::Internal {
    message: String::from("the provider call could not run"),
})]
#[tokio::test]
async fn provider_framework_tests_behavior(#[case] failure: ChatFailure) {
    let layer = ErrorLayer::new().unwrap();
    let call = CallContext {
        id: "call-7",
        include_in_body: true,
        started: Instant::now(),
        errors: &layer,
    };
    let error = DispatchError::Chat { source: failure };
    let response = dispatch_failure(error, &call, None);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let reply: ErrorReply = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply.error.error_type, "internal_server_error");
    assert_eq!(reply.error.code, "500");
    assert_eq!(reply.error.litellm_call_id.as_deref(), Some("call-7"));
}

fn text_chunk(body: &str) -> StreamChunk {
    serde_json::from_str(body).expect("text chunk")
}

async fn bridged(chunks: Vec<&str>, include_usage: bool) -> Vec<Result<Bytes, HttpError>> {
    let items: Vec<_> = chunks
        .into_iter()
        .map(|body| Ok(text_chunk(body)))
        .collect();
    let source = TextSource {
        stream: stream::iter(items).boxed(),
        model: String::from("client"),
        include_usage,
    };
    let mut frames = Vec::new();
    let mut pump = Pump::new(source);
    while let Some((frame, next)) = next_frame::<TextSource, HttpError>(pump).await {
        frames.push(frame);
        pump = next;
    }
    frames
}

// A frame the text bridge wrote, read as the chat chunk the client parses
#[derive(serde::Deserialize)]
struct BridgedFrame {
    object: String,
    model: String,
    choices: Vec<FrameChoice>,
    #[serde(default)]
    usage: Option<BridgedUsage>,
}

#[derive(serde::Deserialize)]
struct BridgedUsage {
    total_tokens: u64,
    #[serde(default)]
    completion_tokens_details: Option<BridgedDetails>,
}

#[derive(serde::Deserialize)]
struct BridgedDetails {
    text_tokens: u64,
}

fn payload(frame: &Result<Bytes, HttpError>) -> BridgedFrame {
    let bytes = frame.as_ref().expect("frame");
    let text = from_utf8(bytes).expect("utf8");
    let body = text.strip_prefix("data: ").expect("data event").trim_end();
    serde_json::from_str(body).expect("frame json")
}

#[tokio::test]
async fn a_text_stream_becomes_chat_chunks_with_a_done_event() {
    let frames = bridged(
        vec![
            r#"{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"}}]}"#,
            r#"{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ],
        false,
    )
    .await;
    assert_eq!(frames.len(), 3);
    let first = payload(&frames[0]);
    assert_eq!(first.object, "chat.completion.chunk");
    assert_eq!(first.model, "client");
    assert_eq!(first.choices[0].delta.content.as_deref(), Some("Hello"));
    assert_eq!(first.choices[0].delta.role.as_deref(), Some("assistant"));
    let second = payload(&frames[1]);
    assert_eq!(second.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(second.choices[0].delta.role, None);
    assert_eq!(second.choices[0].delta.content, None);
    assert_eq!(&**frames[2].as_ref().expect("done"), b"data: [DONE]\n\n");
}

#[tokio::test]
async fn a_text_usage_chunk_needs_the_client_usage_choice() {
    let usage = r#"{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
    let unasked = bridged(vec![usage], false).await;
    assert_eq!(unasked.len(), 1);
    let asked = bridged(vec![usage], true).await;
    let frame = payload(&asked[0]);
    let counted = frame.usage.expect("usage");
    assert_eq!(counted.total_tokens, 5);
    let details = counted.completion_tokens_details.expect("details");
    assert_eq!(details.text_tokens, 2);
}

#[tokio::test]
async fn a_text_provider_failure_is_a_chat_provider_failure() {
    let failure = completion_failure(CompletionFailure {
        status: 429,
        error_type: Some(String::from("rate_limit_error")),
        param: None,
        message: String::from("slow down"),
    });
    let ChatFailure::Route(ChatError::Provider { response }) = failure else {
        panic!("not a provider failure");
    };
    assert_eq!(response.status, 429);
    let reply: ProviderReply = serde_json::from_slice(&response.body).expect("body");
    assert_eq!(reply.error.message, "slow down");
    assert_eq!(reply.error.error_type, "rate_limit_error");
}

#[derive(serde::Deserialize)]
struct ProviderReply {
    error: ProviderFields,
}

#[derive(serde::Deserialize)]
struct ConvertedTextAnswer {
    object: String,
    model: String,
}

const TEXT_CALL_CONFIG: &str = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k}}\n";

const TEXT_CALL_REQUEST: &[u8] =
    br#"{"model":"client","messages":[{"role":"user","content":"hi"}]}"#;

const TEXT_CALL_RESPONSE: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"upstream","choices":[{"text":"Hello","index":0,"logprobs":null,"finish_reason":"stop"}]}"#;

// The text endpoint's answer arrives in the shape the text call asked for; a crossed shape is a dispatch bug.
#[tokio::test]
async fn answered_result_takes_the_buffered_conversion_and_the_stream() {
    let config = config_for(TEXT_CALL_CONFIG);
    let executor = executor_for(&config);
    let request = ChatRequest::parse(TEXT_CALL_REQUEST).expect("chat request");
    let response: CompletionResponse =
        serde_json::from_str(TEXT_CALL_RESPONSE).expect("completion response");
    let buffered = CompletionCallAnswer::Buffered(CompletionAnswer {
        response: Box::new(response),
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::ZERO,
    });
    let ChatResult::Completion(answer) = answered_result(
        &executor,
        TextCallMode::Buffered { convert: true },
        buffered,
        &request,
    )
    .expect("buffered answer") else {
        panic!("a buffered text answer cannot fill a stream");
    };
    let body: ConvertedTextAnswer = serde_json::from_slice(&answer.body).expect("body");
    assert_eq!(body.object, "chat.completion");
    assert_eq!(body.model, "client");
    let empty_stream: CompletionStream = Box::pin(stream::empty());
    let streamed = CompletionCallAnswer::Streamed(CompletionStreamAnswer {
        stream: empty_stream,
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::ZERO,
    });
    let ChatResult::Stream(streamed_answer) =
        answered_result(&executor, TextCallMode::Streamed, streamed, &request)
            .expect("streamed answer")
    else {
        panic!("a text stream cannot fill a buffered answer");
    };
    let frames: Vec<_> = streamed_answer.body.collect().await;
    assert_eq!(frames.len(), 1);
    assert_eq!(&**frames[0].as_ref().expect("done"), b"data: [DONE]\n\n");
}

#[derive(serde::Deserialize)]
struct ProviderFields {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
}
