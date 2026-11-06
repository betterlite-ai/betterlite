#![cfg(test)]

use crate::server::application;
use axum::http::StatusCode;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use std::sync::atomic::AtomicBool;
use tokio::task::JoinSet;

use crate::routes::chat::chat_routes_with_call_id;
use crate::server::chat_setup::chat_executor;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request};
use betterlite_config::loader::ConfigLoader;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::execution::ChatError;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tower::ServiceExt as _;

const ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream-model","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;
const LIMITS: &str = "x-ratelimit-remaining-requests: 9\r\nx-ratelimit-reset-requests: 1s\r\n";
const NO_DELAY: Duration = Duration::ZERO;

struct Exchange {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    sent: String,
}

#[derive(Deserialize)]
struct Completion {
    model: String,
}

#[derive(Deserialize)]
struct ErrorReply {
    error: ErrorFields,
}

#[derive(Deserialize)]
struct ErrorFields {
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
    provider_specific_fields: Option<RouterFields>,
}

#[derive(Deserialize)]
struct RouterFields {
    error: String,
}

async fn provider(listener: TcpListener, reply: (u16, &str), delay: Duration) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(1), listener.accept()).await else {
        return String::new();
    };
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let mut body = vec![0_u8; 4096];
    let read = stream.read(&mut body).await.unwrap();
    sleep(delay).await;
    let (status, answer) = reply;
    let head = format!(
        "HTTP/1.1 {status} X\r\n{LIMITS}content-length: {}\r\n\r\n",
        answer.len()
    );
    let _written = stream.write_all(format!("{head}{answer}").as_bytes()).await;
    body.truncate(read);
    received.extend(body);
    String::from_utf8(received).unwrap()
}

async fn exchange(
    path: &str,
    request: &str,
    call_id: &str,
    reply: (u16, &str),
    (delay, deployment_timeout): (Duration, &str),
) -> Exchange {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', timeout: {deployment_timeout}}}, model_info: {{id: dep-1}}}}\n"
    );
    let environment = BTreeMap::from([
        ("OPENAI_API_KEY".to_owned(), "env-key".to_owned()),
        ("OPENAI_ORGANIZATION".to_owned(), "org-9".to_owned()),
    ]);
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    let config = loader.load(&source).unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let mut builder = Request::post(path).header("content-type", "application/json");
    if !call_id.is_empty() {
        builder = builder.header("x-litellm-call-id", call_id);
    }
    let call = chat_routes_with_call_id(Arc::new(executor), false)
        .unwrap()
        .oneshot(builder.body(Body::from(request.to_owned())).unwrap());
    let (sent, answered) = tokio::join!(provider(listener, reply, delay), call);
    let response = answered.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Exchange {
        status,
        headers,
        body,
        sent,
    }
}

fn header<'map>(headers: &'map HeaderMap, name: &str) -> Option<&'map str> {
    headers.get(name).map(|value| value.to_str().unwrap())
}

#[derive(Deserialize)]
struct MockAnswer {
    model: String,
    choices: Vec<MockChoice>,
    usage: MockUsage,
}

#[derive(Deserialize)]
struct MockChoice {
    message: MockMessage,
}

#[derive(Deserialize)]
struct MockMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<MockToolCallReply>>,
}

#[derive(Deserialize)]
struct MockToolCallReply {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: MockFunctionReply,
}

#[derive(Deserialize)]
struct MockFunctionReply {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct MockUsage {
    #[serde(rename = "prompt_tokens")]
    prompt: u64,
    #[serde(rename = "completion_tokens")]
    completion: u64,
    #[serde(rename = "total_tokens")]
    total: u64,
}

#[tokio::test]
async fn mock_calls_behavior_tests_behavior() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, mock_response: hello, timeout: 0.1}}}}\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap();
    let executor = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(3)).unwrap();
    let request = Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("x-litellm-call-id", "mock-call")
        .body(Body::from(r#"{"model":"g","messages":[]}"#))
        .unwrap();
    let response = chat_routes_with_call_id(Arc::new(executor), false)
        .unwrap()
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 8192).await.unwrap();
    let answer: MockAnswer = serde_json::from_slice(&body).unwrap();
    // a body model restamps the answer, and the stripped deployment model is the fallback
    assert_eq!(answer.model, "g");
    assert_eq!(
        answer
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref()),
        Some("hello")
    );
    assert_eq!(answer.usage.prompt, 10);
    assert_eq!(answer.usage.completion, 20);
    assert_eq!(answer.usage.total, 30);
    assert_eq!(header(&headers, "x-litellm-call-id"), Some("mock-call"));
    assert_eq!(header(&headers, "x-litellm-model-group"), Some("g"));
}

async fn mock_reply(source: &str, path: &str, body: &str) -> (StatusCode, Vec<u8>, HeaderMap) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap();
    let executor = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(3)).unwrap();
    let request = Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = chat_routes_with_call_id(Arc::new(executor), false)
        .unwrap()
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let collected = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, collected.to_vec(), headers)
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
#[case::rate_limit("litellm.RateLimitError", 429, "throttling_error", "429")]
#[case::context_window(
    "litellm.ContextWindowExceededError",
    400,
    "invalid_request_error",
    "400"
)]
#[case::internal_server("litellm.InternalServerError", 500, "internal_server_error", "500")]
#[case::content_filter(
    "Exception: content_filter_policy: blocked",
    400,
    "invalid_request_error",
    "400"
)]
#[case::streaming_error("Exception: mock_streaming_error", 529, "internal_server_error", "529")]
#[tokio::test]
async fn mock_sentinel_tests_behavior(
    #[case] sentinel: &str,
    #[case] status: u16,
    #[case] error_type: &str,
    #[case] code: &str,
) {
    let (observed, body, _headers) = mock_reply(
        &sentinel_source(sentinel),
        "/v1/chat/completions",
        r#"{"model":"g","messages":[]}"#,
    )
    .await;
    assert_eq!(observed.as_u16(), status);
    let reply: ErrorReply = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply.error.error_type, error_type);
    assert_eq!(reply.error.code, code);
}

#[rstest]
#[case::buffered(r#"{"model":"g","messages":[],"n":129}"#)]
#[case::streamed(r#"{"model":"g","messages":[],"stream":true,"n":129}"#)]
#[tokio::test]
async fn a_mock_choice_count_above_the_cap_is_rejected(#[case] body: &str) {
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: hello}}\n";
    let (status, bytes, _headers) = mock_reply(source, "/v1/chat/completions", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let reply: ErrorReply = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply.error.error_type, "invalid_request_error");
    assert_eq!(reply.error.code, "400");
}

#[tokio::test]
async fn a_mock_answers_with_the_stripped_deployment_model() {
    let source = "model_list:\n  - {model_name: text, litellm_params: {model: openai/gpt-4o, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: hi}}\n";
    let (status, bytes, _headers) = mock_reply(
        source,
        "/openai/deployments/text/chat/completions",
        r#"{"messages":[]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let answer: MockAnswer = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(answer.model, "gpt-4o");
}

#[tokio::test]
async fn a_mock_tool_call_fills_litellm_defaults() {
    let source = "model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/up\n      api_base: 'http://127.0.0.1:1'\n      api_key: k\n      mock_response:\n        choices:\n          - message:\n              tool_calls:\n                - function: {name: lookup, arguments: '{\"a\": 1}'}\n";
    let (status, bytes, _headers) = mock_reply(
        source,
        "/v1/chat/completions",
        r#"{"model":"g","messages":[]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let answer: MockAnswer = serde_json::from_slice(&bytes).unwrap();
    let call = answer.choices[0]
        .message
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .expect("tool call");
    assert_eq!(call.kind, "function");
    assert_eq!(call.function.name, "lookup");
    assert_eq!(call.function.arguments, r#"{"a": 1}"#);
    uuid::Uuid::parse_str(&call.id).expect("generated id");
}

#[tokio::test]
async fn a_pinned_model_id_stays_as_declared() {
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: hello}, model_info: {id: my-fixed-id}}\n";
    let (status, _bytes, headers) = mock_reply(
        source,
        "/v1/chat/completions",
        r#"{"model":"g","messages":[]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "x-litellm-model-id"), Some("my-fixed-id"));
}

#[tokio::test]
async fn a_generated_model_id_hashes_the_mock_response() {
    let source = "model_list:\n  - {model_name: text, litellm_params: {model: openai/gpt-4o, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: 'hello world'}}\n";
    let (status, _bytes, headers) = mock_reply(
        source,
        "/v1/chat/completions",
        r#"{"model":"text","messages":[]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header(&headers, "x-litellm-model-id"),
        Some("51b4bb57c0b10832fe43c4dcfb0e49425af84e287cde4b72b07164cfd11a705a")
    );
}

#[rstest]
#[case::shorthand(
    r#"{"model":"g","messages":[],"mock_tool_calls":[{"function":{"arguments":"{}"}}]}"#
)]
#[case::indexed(r#"{"model":"g","messages":[],"mock_tool_calls":[{"index":0,"function":{"name":"f","arguments":"{}"}}]}"#)]
#[case::object(r#"{"model":"g","messages":[],"mock_tool_calls":{"a":1}}"#)]
#[case::no_choice_fields(
    r#"{"model":"g","messages":[],"mock_response":{"choices":[{"logprobs":null}]}}"#
)]
#[case::object_arguments(r#"{"model":"g","messages":[],"mock_response":{"choices":[{"message":{"tool_calls":[{"function":{"name":"f","arguments":{"a":1}}}]}}]}}"#)]
#[tokio::test]
async fn a_stripped_client_mock_field_never_reaches_the_provider(#[case] request: &str) {
    let answered = exchange(
        "/v1/chat/completions",
        request,
        "denied-mock",
        (200, ANSWER),
        (NO_DELAY, "30"),
    )
    .await;
    assert_eq!(answered.status, 200);
    assert!(
        !answered.sent.contains("mock_response"),
        "{}",
        answered.sent
    );
    assert!(
        !answered.sent.contains("mock_tool_calls"),
        "{}",
        answered.sent
    );
}

#[tokio::test]
async fn a_mock_is_keyed_by_its_own_deployment() {
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: answer-a}}\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k, mock_response: answer-b}}\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k}}\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap();
    let executor = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(3)).unwrap();
    let first = &executor.deployments[0];
    let second = &executor.deployments[1];
    let plain = &executor.deployments[2];
    assert_ne!(first.model_id, second.model_id);
    assert_ne!(first.model_id, plain.model_id);
    let request = ChatRequest::parse(br#"{"model":"g","messages":[]}"#).unwrap();
    let first_answer = executor.send(first, &request).await.unwrap();
    assert_eq!(
        first_answer.response.choices[0].message.content.as_deref(),
        Some("answer-a")
    );
    let second_answer = executor.send(second, &request).await.unwrap();
    assert_eq!(
        second_answer.response.choices[0].message.content.as_deref(),
        Some("answer-b")
    );
    assert!(matches!(
        executor.send(plain, &request).await,
        Err(ChatError::Http { .. })
    ));
}

#[rstest]
#[case::chat("/chat/completions", r#"{"model":"g","messages":[]}"#, "g")]
#[case::v1("/v1/chat/completions", r#"{"model":"g","messages":[]}"#, "g")]
#[case::engines_path_wins(
    "/engines/g/chat/completions",
    r#"{"model":"not-configured","messages":[]}"#,
    "not-configured"
)]
#[case::deployments_without_body_model(
    "/openai/deployments/g/chat/completions",
    r#"{"messages":[]}"#,
    "upstream-model"
)]
#[tokio::test]
async fn serves_every_chat_route_with_litellm_headers(
    #[case] path: &str,
    #[case] request: &str,
    #[case] model: &str,
) {
    let answered = exchange(path, request, "call-1", (200, ANSWER), (NO_DELAY, "30")).await;
    assert_eq!(answered.status, 200);
    assert_eq!(
        serde_json::from_slice::<Completion>(&answered.body)
            .unwrap()
            .model,
        model
    );
    assert!(
        answered.sent.contains("authorization: Bearer env-key")
            && answered.sent.contains("openai-organization: org-9")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-call-id"),
        Some("call-1")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-model-id"),
        Some("dep-1")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-model-name"),
        Some("openai/up")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-model-group"),
        Some("g")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-callback-duration-ms"),
        Some("0.0")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-attempted-retries"),
        Some("0")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-attempted-fallbacks"),
        Some("0")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        header(&answered.headers, "llm_provider-x-ratelimit-reset-requests"),
        Some("1s")
    );
    assert_eq!(
        header(&answered.headers, "x-ratelimit-reset-requests"),
        Some("1s")
    );
    assert_eq!(
        header(&answered.headers, "x-ratelimit-remaining-requests"),
        Some("9")
    );
    assert!(
        header(&answered.headers, "x-litellm-model-api-base")
            .unwrap()
            .ends_with("/v1")
    );
    assert!(header(&answered.headers, "x-litellm-timeout").is_none());
    let (whole, fraction) = header(&answered.headers, "x-litellm-response-duration-ms")
        .unwrap()
        .split_once('.')
        .unwrap();
    assert!(whole.parse::<u64>().is_ok() && fraction.len() == 3 && fraction.parse::<u16>().is_ok());
    let (overhead_whole, overhead_fraction) =
        header(&answered.headers, "x-litellm-overhead-duration-ms")
            .unwrap()
            .split_once('.')
            .unwrap();
    assert!(
        overhead_whole.parse::<u64>().is_ok()
            && overhead_fraction.len() == 3
            && overhead_fraction.parse::<u16>().is_ok()
    );
}

#[rstest]
#[case::missing("", 36)]
#[case::too_long(&"c".repeat(257), 36)]
#[case::longest_kept(&"c".repeat(256), 256)]
#[tokio::test]
async fn echoes_a_valid_call_id_and_generates_one_otherwise(
    #[case] call_id: &str,
    #[case] length: usize,
) {
    let answered = exchange(
        "/chat/completions",
        r#"{"model":"g","messages":[]}"#,
        call_id,
        (200, ANSWER),
        (NO_DELAY, "30"),
    )
    .await;
    assert_eq!(
        header(&answered.headers, "x-litellm-call-id")
            .unwrap()
            .len(),
        length
    );
}

#[rstest]
#[case::unknown_model(r#"{"model":"missing","messages":[]}"#, (200, ANSWER, NO_DELAY), (400, "invalid_request_error", None, None))]
#[case::bad_body(r#"{"model":"g"}"#, (200, ANSWER, NO_DELAY), (400, "invalid_request_error", None, None))]
#[case::rate_limited(r#"{"model":"g","messages":[]}"#, (429, r#"{"error":{"type":"requests","param":"p"}}"#, NO_DELAY), (429, "throttling_error", Some("p"), Some("0.4")))]
#[case::slower_than_the_timeout(r#"{"model":"g","messages":[]}"#, (200, ANSWER, Duration::from_secs(2)), (408, "invalid_request_error", None, Some("0.4")))]
#[tokio::test]
async fn answers_failures_in_litellm_shape(
    #[case] request: &str,
    #[case] upstream: (u16, &str, Duration),
    #[case] expected: (u16, &str, Option<&str>, Option<&str>),
) {
    let (reply_status, reply, delay) = upstream;
    let (status, error_type, param, timeout_header) = expected;
    let answered = exchange(
        "/v1/chat/completions",
        request,
        "call-2",
        (reply_status, reply),
        (delay, "0.4"),
    )
    .await;
    assert_eq!(answered.status, status);
    let error = serde_json::from_slice::<ErrorReply>(&answered.body)
        .unwrap()
        .error;
    assert_eq!(error.error_type, error_type);
    assert_eq!(error.code, status.to_string());
    assert_eq!(error.param.as_deref(), param);
    let repeated = error.provider_specific_fields.map(|fields| fields.error);
    assert_eq!(repeated.is_some(), request.contains("missing"));
    assert_eq!(
        header(&answered.headers, "x-litellm-call-id"),
        Some("call-2")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-timeout"),
        timeout_header
    );
    let forwarded = status == 429;
    assert_eq!(
        header(&answered.headers, "x-ratelimit-remaining-requests").is_some(),
        forwarded
    );
    assert_eq!(
        header(&answered.headers, "llm_provider-x-ratelimit-reset-requests").is_some(),
        forwarded
    );
    assert!(header(&answered.headers, "x-ratelimit-reset-requests").is_none());
}

#[path = "tests/stream/tests.rs"]
mod stream;

#[tokio::test]
async fn stream_error_preserves_param() {
    let answered = exchange(
        "/v1/chat/completions",
        r#"{"model":"g","messages":[],"stream":true}"#,
        "stream-error",
        (429, r#"{"error":{"type":"requests","param":"p"}}"#),
        (NO_DELAY, "0.4"),
    )
    .await;
    assert_eq!(answered.status, 429);
    let error = serde_json::from_slice::<ErrorReply>(&answered.body)
        .unwrap()
        .error;
    assert_eq!(error.error_type, "throttling_error");
    assert_eq!(error.param.as_deref(), Some("p"));
    assert_eq!(error.code, "429");
}

#[rstest]
#[case::provider_done("data: [DONE]\n\n")]
#[case::duplicate_done("data: [DONE]\n\ndata: [DONE]\n\n")]
#[case::provider_eof("")]
#[case::partial_event("data: {\"id\":")]
#[tokio::test]
async fn stream_finishes_once(#[case] input: &str) {
    let answered = exchange(
        "/v1/chat/completions",
        r#"{"model":"g","messages":[],"stream":true}"#,
        "stream-done",
        (200, input),
        (NO_DELAY, "30"),
    )
    .await;
    assert_eq!(answered.body, b"data: [DONE]\n\n");
    assert_eq!(
        header(&answered.headers, "content-type"),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(header(&answered.headers, "cache-control"), Some("no-cache"));
    assert_eq!(header(&answered.headers, "x-accel-buffering"), Some("no"));
    assert_eq!(
        header(&answered.headers, "x-litellm-call-id"),
        Some("stream-done")
    );
}

#[rstest]
#[case::model_id("x-litellm-model-id", "dep-1")]
#[case::model_name("x-litellm-model-name", "openai/up")]
#[case::model_group("x-litellm-model-group", "g")]
#[case::version("x-litellm-version", env!("CARGO_PKG_VERSION"))]
#[case::callback("x-litellm-callback-duration-ms", "0.0")]
#[case::retries("x-litellm-attempted-retries", "0")]
#[case::fallbacks("x-litellm-attempted-fallbacks", "0")]
#[tokio::test]
async fn stream_has_success_headers(#[case] name: &str, #[case] value: &str) {
    let answered = exchange(
        "/v1/chat/completions",
        r#"{"model":"g","messages":[],"stream":true}"#,
        "stream-headers",
        (200, "data: [DONE]\n\n"),
        (NO_DELAY, "30"),
    )
    .await;
    assert_eq!(header(&answered.headers, name), Some(value));
    assert!(
        header(&answered.headers, "x-litellm-model-api-base")
            .unwrap()
            .ends_with("/v1")
    );
    assert!(
        header(&answered.headers, "x-litellm-response-duration-ms")
            .unwrap()
            .parse::<rust_decimal::Decimal>()
            .unwrap()
            >= rust_decimal::Decimal::ZERO
    );
    assert!(
        header(&answered.headers, "x-litellm-overhead-duration-ms")
            .unwrap()
            .parse::<rust_decimal::Decimal>()
            .unwrap()
            >= rust_decimal::Decimal::ZERO
    );
}

#[tokio::test]
async fn a_streaming_success_survives_a_one_second_provider_delay() {
    let answered = exchange(
        "/v1/chat/completions",
        r#"{"model":"g","messages":[],"stream":true}"#,
        "slow-stream",
        (200, "data: [DONE]\n\n"),
        (Duration::from_secs(1), "30"),
    )
    .await;
    assert_eq!(answered.status, 200);
    assert_eq!(answered.body, b"data: [DONE]\n\n");
    let expected = [
        ("x-litellm-call-id", "slow-stream"),
        ("x-litellm-model-id", "dep-1"),
        ("x-litellm-model-name", "openai/up"),
        ("x-litellm-model-group", "g"),
        ("x-litellm-version", env!("CARGO_PKG_VERSION")),
        ("x-litellm-callback-duration-ms", "0.0"),
        ("x-litellm-attempted-retries", "0"),
        ("x-litellm-attempted-fallbacks", "0"),
        ("content-type", "text/event-stream; charset=utf-8"),
        ("cache-control", "no-cache"),
        ("x-accel-buffering", "no"),
        ("x-ratelimit-remaining-requests", "9"),
        ("x-ratelimit-reset-requests", "1s"),
        ("llm_provider-x-ratelimit-remaining-requests", "9"),
        ("llm_provider-x-ratelimit-reset-requests", "1s"),
    ];
    assert!(
        expected
            .iter()
            .all(|(name, value)| header(&answered.headers, name) == Some(value))
    );
    assert!(header(&answered.headers, "x-litellm-timeout").is_none());
    assert!(
        header(&answered.headers, "x-litellm-model-api-base")
            .is_some_and(|base| base.ends_with("/v1"))
    );
    assert!(header(&answered.headers, "x-litellm-response-duration-ms").is_some());
    assert!(header(&answered.headers, "x-litellm-overhead-duration-ms").is_some());
}

async fn normalization_provider(upstream: TcpListener) {
    let mock = axum::Router::new().fallback(|| async {
            (StatusCode::TOO_MANY_REQUESTS,
             [("retry-after", "5"), ("x-ratelimit-limit-requests", "100"), ("x-ratelimit-remaining-requests", "9"), ("x-ratelimit-limit-tokens", "1000"), ("x-ratelimit-remaining-tokens", "99"), ("x-ratelimit-reset-requests", "1s"), ("set-cookie", "secret-cookie"), ("access-control-allow-origin", "https://evil.example"), ("llm_provider-custom", "kept")],
             r#"{"error":{"message":"Bearer sk-secretsecretsecretsecret at 10.0.0.1 /home/private/file","litellm_call_id":"forged","type":"rate_limit_error","param":"Bearer sk-secretsecretsecretsecret"}}"#)
        });
    axum::serve(upstream, mock).await.unwrap();
}

async fn normalization_gateway(enabled: bool) -> (ProxiedClient, String, JoinSet<()>) {
    let mut tasks = JoinSet::new();
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = format!(
        "general_settings: {{include_call_id_in_error_body: {enabled}}}\nmodel_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_key: test-key, api_base: 'http://{}'}}}}",
        upstream.local_addr().unwrap()
    );
    tasks.spawn(normalization_provider(upstream));
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source)
    .unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let application = application(&config, executor, Arc::new(AtomicBool::new(false))).unwrap();
    tasks.spawn(async move {
        axum::serve(listener, application).await.unwrap();
    });
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    (ProxiedClient::outbound(&policy).unwrap(), url, tasks)
}

#[rstest]
#[case::disabled(false, "client-id", r#"{"model":"g","messages":[]}"#, (429, false))]
#[case::client_id(true, "exact-client-id", r#"{"model":"g","messages":[]}"#, (429, true))]
#[case::generated(true, "", r#"{"model":"g","messages":[]}"#, (429, true))]
#[case::too_long(true, &"x".repeat(257), r#"{"model":"g","messages":[]}"#, (429, true))]
#[case::maximum_length(true, &"x".repeat(256), r#"{"model":"g","messages":[]}"#, (429, true))]
#[case::stream_error(true, "stream-id", r#"{"model":"g","messages":[],"stream":true}"#, (429, true))]
#[case::parse_error(true, "parse-id", "{", (400, false))]
#[case::syntax_error(true, "syntax-id", "!", (400, false))]
#[case::empty_body(true, "empty-id", "", (400, true))]
#[case::schema_error(true, "schema-id", r#"{"model":"g"}"#, (400, true))]
#[case::unsupported_parameter(true, "unsupported-id", r#"{"model":"g","messages":[],"mock_timeout":true}"#, (400, true))]
#[case::unknown_model(
    true,
    "router-id",
    r#"{"model":"Bearer sk-secretsecretsecretsecret","messages":[]}"#,
    (400, true)
)]
#[tokio::test]
async fn error_normalization_tests_behavior(
    #[case] enabled: bool,
    #[case] id: &str,
    #[case] request: &str,
    #[case] expected: (u16, bool),
) {
    let (status, include_id) = expected;
    let (client, url, mut tasks) = normalization_gateway(enabled).await;
    let response = client
        .client
        .post(url)
        .header("content-type", "application/json")
        .header("x-litellm-call-id", id)
        .body(request.to_owned())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), status);
    let headers = response.headers().clone();
    let actual_id = header(&headers, "x-litellm-call-id").unwrap();
    if id.is_empty() || id.len() > 256 {
        uuid::Uuid::parse_str(actual_id).unwrap();
    } else {
        assert_eq!(actual_id, id);
    }
    let bytes = response.bytes().await.unwrap();
    let body = serde_json::from_slice::<CallIdReply>(&bytes).unwrap();
    assert_eq!(
        body.error.litellm_call_id.as_deref(),
        include_id.then_some(actual_id)
    );
    assert!(body.error.normalized_error.is_none());
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!text.contains("sk-secretsecretsecretsecret"));
    assert!(!text.contains("10.0.0.1"));
    assert!(!text.contains("/home/private"));
    assert!(!text.contains("forged"));
    if status == 429 {
        assert_failure_headers(&headers);
    }
    tasks.shutdown().await;
}

#[derive(Deserialize)]
struct CallIdReply {
    error: CallIdFields,
}
#[derive(Deserialize)]
struct CallIdFields {
    litellm_call_id: Option<String>,
    normalized_error: Option<String>,
}

fn assert_failure_headers(headers: &HeaderMap) {
    assert_eq!(header(headers, "llm_provider-retry-after"), Some("5"));
    assert_eq!(header(headers, "llm_provider-custom"), Some("kept"));
    assert_eq!(header(headers, "x-ratelimit-limit-requests"), Some("100"));
    assert_eq!(header(headers, "x-ratelimit-remaining-requests"), Some("9"));
    assert_eq!(header(headers, "x-ratelimit-limit-tokens"), Some("1000"));
    assert_eq!(header(headers, "x-ratelimit-remaining-tokens"), Some("99"));
    assert_eq!(
        header(headers, "llm_provider-x-ratelimit-reset-requests"),
        Some("1s")
    );
    assert!(header(headers, "x-ratelimit-reset-requests").is_none());
    assert!(header(headers, "set-cookie").is_none());
    assert!(header(headers, "llm_provider-set-cookie").is_none());
    assert!(header(headers, "access-control-allow-origin").is_none());
    assert!(header(headers, "llm_provider-access-control-allow-origin").is_none());
}
