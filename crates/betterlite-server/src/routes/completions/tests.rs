#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::Response;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tower::ServiceExt as _;

use super::routes;
use crate::route_read_through::ModelReadThrough;
use crate::routes::router;
use crate::routes::tests::state_for;
use crate::server::chat_setup::chat_executor;
use crate::state::ServerState;
use crate::state::shared::Shared;

const ANSWER: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-3.5-turbo-instruct","choices":[{"text":" hello","index":0,"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

const CHAT_ANSWER: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":" hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

const CHAT_STREAM: &str = concat!(
    "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\" hello\"},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" there\"},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n"
);

const STREAM: &str = concat!(
    "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":1700000000,\"model\":\"gpt-3.5-turbo-instruct\",\"choices\":[{\"text\":\" hello\",\"index\":0,\"logprobs\":null,\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":1700000000,\"model\":\"gpt-3.5-turbo-instruct\",\"choices\":[{\"text\":\" there\",\"index\":0,\"logprobs\":null,\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n"
);

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config")
}

fn state(base: &str) -> Arc<ServerState> {
    let source = format!(
        "model_list:\n  - model_name: text-model\n    litellm_params: {{model: openai/gpt-3.5-turbo-instruct, api_base: '{base}', api_key: oracle-key}}\n"
    );
    state_for(&config(&source))
}

// a chat-catalog deployment the completion route reaches through the chat endpoint
fn chat_state(base: &str) -> Arc<ServerState> {
    let source = format!(
        "model_list:\n  - model_name: chat-model\n    litellm_params: {{model: openai/gpt-4o, api_base: '{base}', api_key: oracle-key}}\n"
    );
    state_for(&config(&source))
}

async fn chat_completion(body: &str, upstream: &str) -> Answered {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let state = chat_state(&format!("http://{address}/v1"));
    answered(state, listener, "/v1/completions", body, (200, upstream)).await
}

async fn serve(listener: TcpListener, status: u16, body: &str) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(2), listener.accept()).await else {
        return String::new();
    };
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        let Ok(byte) = stream.read_u8().await else {
            return String::from_utf8(received).unwrap_or_default();
        };
        received.push(byte);
    }
    let mut payload = vec![0_u8; 8192];
    let read = stream.read(&mut payload).await.unwrap_or(0);
    payload.truncate(read);
    received.extend(payload);
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\ncontent-type: application/json\r\nx-request-id: provider-call\r\n\r\n",
        body.len()
    );
    let _written = stream.write_all(format!("{head}{body}").as_bytes()).await;
    String::from_utf8(received).unwrap_or_default()
}

#[derive(Debug, Default, Deserialize)]
struct AnswerBody {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    choices: Vec<AnswerChoice>,
    #[serde(default)]
    error: Option<AnswerError>,
}

#[derive(Debug, Default, Deserialize)]
struct AnswerChoice {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AnswerError {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

struct Answered {
    status: StatusCode,
    headers: HeaderMap,
    body: AnswerBody,
    raw: String,
    sent: String,
}

async fn call(path: &str, body: &str, upstream_status: u16, upstream: &str) -> Answered {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let state = state(&format!("http://{address}/v1"));
    answered(state, listener, path, body, (upstream_status, upstream)).await
}

async fn call_with_call_id_in_body(body: &str, include: bool) -> Answered {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let source = format!(
        "general_settings: {{include_call_id_in_error_body: {include}}}\nmodel_list:\n  - model_name: text-model\n    litellm_params: {{model: openai/gpt-3.5-turbo-instruct, api_base: 'http://{address}/v1', api_key: oracle-key}}\n"
    );
    let state = state_for(&config(&source));
    answered(state, listener, "/v1/completions", body, (500, "")).await
}

async fn answered(
    state: Arc<ServerState>,
    listener: TcpListener,
    path: &str,
    body: &str,
    provider: (u16, &str),
) -> Answered {
    let (provider_status, provider_body) = provider;
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer oracle-master")
        .header("x-litellm-call-id", "caller-id")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let (request_result, sent) = tokio::join!(
        routes(&state).oneshot(request),
        serve(listener, provider_status, provider_body)
    );
    let response = request_result.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.expect("body");
    let raw = str::from_utf8(&bytes).expect("utf8 body").to_owned();
    Answered {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or_default(),
        raw,
        sent,
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    StringPrompt,
    StringsPrompt,
    TokensPrompt,
    TokenListsPrompt,
    EmptyStringPrompt,
    EmptyListRejected,
    MixedPromptRejected,
    UnversionedPath,
    EnginesPath,
    DeploymentsPath,
    ParametersAndControls,
    ProviderBadRequest,
    ProviderRateLimit,
    StreamTrue,
    StreamFalse,
    AuthRequired,
}

#[rstest]
#[case::string_prompt(Scenario::StringPrompt)]
#[case::strings_prompt(Scenario::StringsPrompt)]
#[case::tokens_prompt(Scenario::TokensPrompt)]
#[case::token_lists_prompt(Scenario::TokenListsPrompt)]
#[case::empty_string_prompt(Scenario::EmptyStringPrompt)]
#[case::empty_list_rejected(Scenario::EmptyListRejected)]
#[case::mixed_prompt_rejected(Scenario::MixedPromptRejected)]
#[case::unversioned_path(Scenario::UnversionedPath)]
#[case::engines_path(Scenario::EnginesPath)]
#[case::deployments_path(Scenario::DeploymentsPath)]
#[case::parameters_and_controls(Scenario::ParametersAndControls)]
#[case::provider_bad_request(Scenario::ProviderBadRequest)]
#[case::provider_rate_limit(Scenario::ProviderRateLimit)]
#[case::stream_true(Scenario::StreamTrue)]
#[case::stream_false(Scenario::StreamFalse)]
#[case::auth_required(Scenario::AuthRequired)]
#[tokio::test]
async fn text_completions_text_completion_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::StringPrompt => string_prompt().await,
        Scenario::StringsPrompt => strings_prompt().await,
        Scenario::TokensPrompt => tokens_prompt().await,
        Scenario::TokenListsPrompt => token_lists_prompt().await,
        Scenario::EmptyStringPrompt => empty_string_prompt().await,
        Scenario::EmptyListRejected => empty_list_rejected().await,
        Scenario::MixedPromptRejected => mixed_prompt_rejected().await,
        Scenario::UnversionedPath => alias("/completions", "text-model").await,
        Scenario::EnginesPath => alias("/engines/text-model/completions", "wrong").await,
        Scenario::DeploymentsPath => {
            alias("/openai/deployments/text-model/completions", "wrong").await;
        }
        Scenario::ParametersAndControls => parameters_and_controls().await,
        Scenario::ProviderBadRequest => provider_error(400, None).await,
        Scenario::ProviderRateLimit => provider_error(429, Some("throttling_error")).await,
        Scenario::StreamTrue => stream_true().await,
        Scenario::StreamFalse => stream_false().await,
        Scenario::AuthRequired => auth_required().await,
    }
}

async fn text_answer(path: &str, prompt: &str) -> Answered {
    let body = format!(r#"{{"model":"text-model","prompt":{prompt}}}"#);
    call(path, &body, 200, ANSWER).await
}

async fn string_prompt() {
    let answered = text_answer("/v1/completions", r#""Once upon a time""#).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        answered
            .body
            .choices
            .first()
            .and_then(|choice| choice.text.as_deref()),
        Some(" hello")
    );
    assert_eq!(answered.body.object.as_deref(), Some("text_completion"));
    assert_eq!(answered.body.model.as_deref(), Some("text-model"));
    assert_eq!(
        answered
            .headers
            .get("x-litellm-call-id")
            .and_then(|value| value.to_str().ok()),
        Some("caller-id")
    );
    assert!(
        answered.sent.starts_with("POST /v1/completions "),
        "{}",
        answered.sent
    );
    // the completion handler leaves get_custom_headers with no provider headers, so a 200 carries none of the provider's
    assert!(
        !answered.headers.contains_key("llm_provider-x-request-id"),
        "{:?}",
        answered.headers
    );
    assert!(
        !answered.headers.contains_key("llm_provider-content-type"),
        "{:?}",
        answered.headers
    );
}

fn assert_prompt_sent(answered: &Answered, expected: &str) {
    assert_eq!(answered.status, StatusCode::OK);
    assert!(answered.sent.contains(expected), "{}", answered.sent);
}

async fn strings_prompt() {
    let answered = text_answer("/v1/completions", r#"["a","b"]"#).await;
    assert_prompt_sent(&answered, r#""prompt":["a","b"]"#);
}

async fn tokens_prompt() {
    let answered = text_answer("/v1/completions", "[1,2]").await;
    assert_prompt_sent(&answered, r#""prompt":[1,2]"#);
}

async fn token_lists_prompt() {
    let answered = text_answer("/v1/completions", "[[1,2],[3]]").await;
    assert_prompt_sent(&answered, r#""prompt":[[1,2],[3]]"#);
}

async fn empty_string_prompt() {
    let answered = text_answer("/v1/completions", r#""""#).await;
    assert_prompt_sent(&answered, r#""prompt":"""#);
}

async fn empty_list_rejected() {
    let answered = text_answer("/v1/completions", "[]").await;
    assert_eq!(answered.status, StatusCode::BAD_REQUEST);
    assert!(
        answered.raw.contains(r#""type":"invalid_request_error""#),
        "{}",
        answered.raw
    );
    assert_eq!(
        answered
            .body
            .error
            .as_ref()
            .and_then(|error| error.code.as_deref()),
        Some("400")
    );
    assert!(answered.sent.is_empty(), "no provider request");
}

async fn mixed_prompt_rejected() {
    let answered = text_answer("/v1/completions", r#"[["a"]]"#).await;
    assert_eq!(answered.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        answered
            .body
            .error
            .as_ref()
            .and_then(|error| error.kind.as_deref()),
        Some("internal_server_error")
    );
    assert!(answered.sent.is_empty(), "no provider request");
}

async fn alias(path: &str, model: &str) {
    let body = format!(r#"{{"model":"{model}","prompt":"hello"}}"#);
    let answered = call(path, &body, 200, ANSWER).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert!(
        answered
            .sent
            .contains(r#""model":"gpt-3.5-turbo-instruct""#),
        "{}",
        answered.sent
    );
    // the unversioned path resolves the model in the body; the two alias paths pass theirs in the path
    assert_eq!(answered.body.model.as_deref(), Some(model));
}

// matchit 0.8 refuses a catch-all with a literal suffix, so the alias paths take one model segment
#[tokio::test]
async fn a_slash_model_answers_not_found_on_the_alias_paths() {
    let answered = call(
        "/engines/openai/text-model/completions",
        r#"{"model":"text-model","prompt":"hello"}"#,
        200,
        ANSWER,
    )
    .await;
    assert_eq!(answered.status, StatusCode::NOT_FOUND);
    assert!(answered.sent.is_empty(), "no provider request");
}

async fn parameters_and_controls() {
    let body = r#"{"model":"text-model","prompt":"hi","temperature":0.7,"logprobs":2,"top_logprobs":1,"max_retries":5,"api_base":"http://evil.example","api_key":"sk-evil","api_version":"2099-01-01","metadata":{"user":"x"},"mock_response":"not a mock","mock_timeout":true,"num_retries":3,"timeout":1,"extra_headers":{"x-caller":"yes"}}"#;
    let answered = call("/v1/completions", body, 200, ANSWER).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_sent_body(
        &answered,
        &[
            r#""temperature":0.7"#,
            r#""logprobs":2"#,
            r#""top_logprobs":1"#,
        ],
        true,
    );
    assert_sent_body(
        &answered,
        &[
            "max_retries",
            "api_base",
            "api_key",
            "api_version",
            "metadata",
            "mock_response",
            "mock_timeout",
            "num_retries",
            "sk-evil",
            "evil.example",
            "2099-01-01",
            "not a mock",
        ],
        false,
    );
    assert_upstream_request_headers(&answered);
}

fn assert_sent_body(answered: &Answered, needles: &[&str], expected_present: bool) {
    for needle in needles {
        assert_eq!(
            answered.sent.contains(needle),
            expected_present,
            "{needle} expected present={expected_present}: {}",
            answered.sent
        );
    }
}

fn assert_upstream_request_headers(answered: &Answered) {
    let sent = answered.sent.to_ascii_lowercase();
    for expected in [
        "authorization: bearer oracle-key",
        "content-type: application/json",
        "x-caller: yes",
    ] {
        assert!(
            sent.contains(expected),
            "{expected} missing: {}",
            answered.sent
        );
    }
}

async fn provider_error(status: u16, error_type: Option<&str>) {
    let upstream = r#"{"error":{"message":"provider refused","type":"invalid_request_error","param":"prompt","code":"bad"}}"#;
    let body = r#"{"model":"text-model","prompt":"hi"}"#;
    let answered = call("/v1/completions", body, status, upstream).await;
    assert_eq!(answered.status.as_u16(), status);
    assert_eq!(
        answered
            .body
            .error
            .as_ref()
            .and_then(|error| error.kind.as_deref()),
        error_type
    );
    let code = status.to_string();
    assert_eq!(
        answered
            .body
            .error
            .as_ref()
            .and_then(|error| error.code.as_deref()),
        Some(code.as_str())
    );
    // proxy_server.py's completion except answers with litellm_call_id_headers alone, so the provider's x-request-id never comes back
    assert!(
        !answered.headers.contains_key("llm_provider-x-request-id"),
        "{:?}",
        answered.headers
    );
    assert!(
        !answered.headers.contains_key("x-litellm-timeout"),
        "{:?}",
        answered.headers
    );
    assert_eq!(
        answered
            .headers
            .get("x-litellm-call-id")
            .and_then(|value| value.to_str().ok()),
        Some("caller-id")
    );
}

async fn stream_false() {
    let body = r#"{"model":"text-model","prompt":"hi","stream":false}"#;
    let answered = call("/v1/completions", body, 200, ANSWER).await;
    assert_eq!(answered.status, StatusCode::OK);
    // LiteLLM puts stream in optional_params whenever it is not None, so the provider body keeps the explicit false
    assert!(
        answered.sent.contains(r#""stream":false"#),
        "{}",
        answered.sent
    );
}

async fn stream_true() {
    let body = r#"{"model":"text-model","prompt":"hi","stream":true}"#;
    let answered = call("/v1/completions", body, 200, STREAM).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        answered
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    assert!(
        answered.sent.contains(r#""stream":true"#),
        "{}",
        answered.sent
    );
    assert!(
        answered.raw.contains(r#""object":"text_completion""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.contains(r#""text":" hello""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.contains(r#""text":" there""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.ends_with("data: [DONE]\n\n"),
        "{:?}",
        answered.raw
    );
}

#[tokio::test]
async fn a_chat_deployment_answers_the_text_shape() {
    let answered = chat_completion(r#"{"model":"chat-model","prompt":"hi"}"#, CHAT_ANSWER).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        answered
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(answered.body.object.as_deref(), Some("text_completion"));
    assert_eq!(answered.body.model.as_deref(), Some("chat-model"));
    assert_eq!(
        answered
            .body
            .choices
            .first()
            .and_then(|choice| choice.text.as_deref()),
        Some(" hello")
    );
    // the chat endpoint receives the messages the text prompt became, and a buffered call sends no stream flag
    assert!(
        answered.sent.contains(r#""role":"user""#),
        "{}",
        answered.sent
    );
    assert!(!answered.sent.contains(r#""stream""#), "{}", answered.sent);
}

#[tokio::test]
async fn a_chat_deployment_streams_the_text_shape() {
    let answered = chat_completion(
        r#"{"model":"chat-model","prompt":"hi","stream":true}"#,
        CHAT_STREAM,
    )
    .await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        answered
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    assert!(
        answered.sent.contains(r#""stream":true"#),
        "{}",
        answered.sent
    );
    assert!(
        answered.raw.contains(r#""object":"text_completion""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.contains(r#""text":" hello""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.contains(r#""text":" there""#),
        "{}",
        answered.raw
    );
    assert!(
        answered.raw.ends_with("data: [DONE]\n\n"),
        "{:?}",
        answered.raw
    );
}

#[tokio::test]
async fn error_bodies_carry_the_call_id_when_the_setting_is_on() {
    let enabled = call_with_call_id_in_body(r#"{"model":"text-model","prompt":[]}"#, true).await;
    assert_eq!(enabled.status, StatusCode::BAD_REQUEST);
    assert!(
        enabled.raw.contains(r#""litellm_call_id":"caller-id""#),
        "{}",
        enabled.raw
    );
    assert_eq!(
        enabled
            .headers
            .get("x-litellm-call-id")
            .and_then(|value| value.to_str().ok()),
        Some("caller-id")
    );

    let disabled = call_with_call_id_in_body(r#"{"model":"text-model","prompt":[]}"#, false).await;
    assert!(
        !disabled.raw.contains("litellm_call_id"),
        "{}",
        disabled.raw
    );
}

async fn assert_refusal_keeps_the_call_id(body: &str, present: bool) {
    let answered = call_with_call_id_in_body(body, true).await;
    assert_eq!(answered.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        answered.raw.contains("litellm_call_id"),
        present,
        "{}",
        answered.raw
    );
}

#[tokio::test]
async fn error_bodies_carry_the_call_id_unless_the_json_is_malformed() {
    // LiteLLM never parses this body, so it has no call id to put in the error body
    assert_refusal_keeps_the_call_id("{", false).await;
    // an empty body counts as {}, the way LiteLLM reads it, so its refusal keeps the id
    assert_refusal_keeps_the_call_id("", true).await;
    // a body that parses as JSON but not as a completion request is read before LiteLLM builds the call id, so the id stays
    assert_refusal_keeps_the_call_id(
        r#"{"model":"text-model","prompt":"hi","stream":"yes"}"#,
        true,
    )
    .await;
    // this body parsed, so its refusal carries the id LiteLLM had already resolved
    assert_refusal_keeps_the_call_id(
        r#"{"model":"text-model","prompt":"hi","fallbacks":["b"]}"#,
        true,
    )
    .await;
}

async fn auth_required() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let source = format!(
        "general_settings: {{master_key: oracle-master}}\nmodel_list:\n  - model_name: text-model\n    litellm_params: {{model: openai/gpt-3.5-turbo-instruct, api_base: 'http://{address}/v1', api_key: oracle-key}}\n"
    );
    let state = state_for(&config(&source));
    let application = || {
        router(
            Arc::clone(&state),
            Arc::new(AtomicBool::new(false)),
            Router::new(),
            Router::new(),
        )
    };
    let body = r#"{"model":"text-model","prompt":"hi"}"#;
    let unauthorized = application()
        .oneshot(
            Request::post("/v1/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_missing_key_is_internal_server_error(unauthorized).await;
    // the authorized call reaches the provider, so its future runs beside the fake provider
    let authorized = application().oneshot(
        Request::post("/v1/completions")
            .header("content-type", "application/json")
            .header("authorization", "Bearer oracle-master")
            .body(Body::from(body))
            .expect("request"),
    );
    let (response, sent) = tokio::join!(authorized, serve(listener, 200, ANSWER));
    assert_eq!(response.expect("response").status(), StatusCode::OK);
    assert!(sent.starts_with("POST /v1/completions "), "{sent}");
}

// LiteLLM raises a plain exception for a key it never got, and its server answers that with FastAPI's 500 text
async fn assert_missing_key_is_internal_server_error(response: Response) {
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), 1 << 20).await.expect("body");
    assert_eq!(
        str::from_utf8(&body).expect("utf8 body"),
        "Internal Server Error"
    );
}

struct RecordingReadThrough {
    answer: bool,
    asked: Mutex<Vec<String>>,
}

impl RecordingReadThrough {
    fn new(answer: bool) -> Arc<Self> {
        Arc::new(Self {
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl ModelReadThrough for RecordingReadThrough {
    async fn attempt_model(&self, model: &str) -> bool {
        self.asked.lock().unwrap().push(model.to_owned());
        self.answer
    }
}

#[derive(Deserialize)]
struct NotFoundReply {
    error: NotFoundFields,
}

#[derive(Deserialize)]
struct NotFoundFields {
    message: String,
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

fn litellm_text(model: &str) -> String {
    format!(
        "400: {{'error': '/completions: Invalid model name passed in model={model}. Call `/v1/models` to view available models for your key.'}}"
    )
}

fn state_with_read_through(read_through: Option<Arc<dyn ModelReadThrough>>) -> Arc<ServerState> {
    let source = "model_list:\n  - model_name: text-model\n    litellm_params: {model: openai/gpt-3.5-turbo-instruct, api_base: 'http://127.0.0.1:9/v1', api_key: oracle-key}\n";
    let config = config(source);
    let executor =
        chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).expect("executor");
    let shared = Arc::new(Shared {
        bundled_catalog: Arc::clone(&executor.bundled_catalog),
    });
    let mut state = ServerState::new(&config, Arc::new(executor), shared);
    state.model_read_through.read_through = read_through;
    Arc::new(state)
}

async fn ask(
    path: &str,
    body: &str,
    read_through: Option<Arc<dyn ModelReadThrough>>,
) -> (StatusCode, NotFoundReply) {
    let state = state_with_read_through(read_through);
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer oracle-master")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let response = routes(&state).oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.expect("body");
    (status, serde_json::from_slice(&bytes).expect("error body"))
}

#[tokio::test]
async fn unknown_model_answers_litellm_text_on_the_v1_path() {
    let (status, reply) = ask("/v1/completions", r#"{"model":"nope","prompt":"hi"}"#, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let text = litellm_text("nope");
    assert_eq!(reply.error.message, text);
    assert_eq!(reply.error.error_type, "None");
    assert_eq!(reply.error.param.as_deref(), Some("None"));
    assert_eq!(reply.error.code, "400");
    assert_eq!(
        reply
            .error
            .provider_specific_fields
            .map(|fields| fields.error),
        None
    );
}

#[tokio::test]
async fn unknown_model_answers_litellm_text_on_an_engines_path() {
    let (status, reply) = ask(
        "/engines/nope/completions",
        r#"{"model":"body-model","prompt":"hi"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error.message, litellm_text("nope"));
}

#[tokio::test]
async fn unknown_model_retries_once_when_the_read_through_answers() {
    let recorder = RecordingReadThrough::new(true);
    let port: Arc<dyn ModelReadThrough> = Arc::clone(&recorder);
    let (status, reply) = ask(
        "/v1/completions",
        r#"{"model":"nope","prompt":"hi"}"#,
        Some(port),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error.message, litellm_text("nope"));
    assert_eq!(recorder.asked(), vec![String::from("nope")]);
}

#[tokio::test]
async fn unknown_model_keeps_the_first_answer_when_the_read_through_declines() {
    let recorder = RecordingReadThrough::new(false);
    let port: Arc<dyn ModelReadThrough> = Arc::clone(&recorder);
    let (status, reply) = ask(
        "/v1/completions",
        r#"{"model":"nope","prompt":"hi"}"#,
        Some(port),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error.message, litellm_text("nope"));
    assert_eq!(recorder.asked(), vec![String::from("nope")]);
}

#[tokio::test]
async fn unknown_model_without_a_model_never_asks_the_read_through() {
    let recorder = RecordingReadThrough::new(true);
    let port: Arc<dyn ModelReadThrough> = Arc::clone(&recorder);
    let (status, reply) = ask("/v1/completions", r#"{"prompt":"hi"}"#, Some(port)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error.message, litellm_text("None"));
    assert!(recorder.asked().is_empty());
}

#[tokio::test]
async fn unknown_model_redacts_a_key_like_model() {
    let (status, reply) = ask(
        "/v1/completions",
        r#"{"model":"Bearer sk-secretsecretsecretsecret","prompt":"hi"}"#,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let repeated = reply
        .error
        .provider_specific_fields
        .map(|fields| fields.error);
    assert_eq!(repeated, None);
    assert!(!reply.error.message.contains("sk-secretsecretsecretsecret"));
}
