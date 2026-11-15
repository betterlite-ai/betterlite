#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use rstest::rstest;
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tower::ServiceExt as _;

use super::routes;
use crate::state::ServerState;

const ANSWER: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-3.5-turbo-instruct","choices":[{"text":" hello","index":0,"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

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
    crate::routes::tests::state_for(&config(&source))
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
    let state = crate::routes::tests::state_for(&config(&source));
    answered(state, listener, "/v1/completions", body, (500, "")).await
}

async fn answered(
    state: Arc<ServerState>,
    listener: TcpListener,
    path: &str,
    body: &str,
    upstream: (u16, &str),
) -> Answered {
    let (upstream_status, upstream) = upstream;
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer oracle-master")
        .header("x-litellm-call-id", "caller-id")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let (response, sent) = tokio::join!(
        routes(&state).oneshot(request),
        serve(listener, upstream_status, upstream)
    );
    let response = response.expect("response");
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
    StreamNotBuilt,
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
#[case::stream_not_built(Scenario::StreamNotBuilt)]
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
        Scenario::UnversionedPath => alias("/completions").await,
        Scenario::EnginesPath => alias("/engines/text-model/completions").await,
        Scenario::DeploymentsPath => alias("/openai/deployments/text-model/completions").await,
        Scenario::ParametersAndControls => parameters_and_controls().await,
        Scenario::ProviderBadRequest => provider_error(400, None).await,
        Scenario::ProviderRateLimit => provider_error(429, Some("throttling_error")).await,
        Scenario::StreamNotBuilt => stream_not_built().await,
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

async fn alias(path: &str) {
    let body = r#"{"model":"wrong","prompt":"hello"}"#;
    let answered = call(path, body, 200, ANSWER).await;
    assert_eq!(answered.status, StatusCode::OK);
    assert!(
        answered
            .sent
            .contains(r#""model":"gpt-3.5-turbo-instruct""#),
        "{}",
        answered.sent
    );
    assert_eq!(answered.body.model.as_deref(), Some("wrong"));
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
    assert!(
        answered.sent.contains(r#""temperature":0.7"#),
        "{}",
        answered.sent
    );
    assert!(
        answered.sent.contains(r#""logprobs":2"#),
        "{}",
        answered.sent
    );
    assert!(
        answered.sent.contains(r#""top_logprobs":1"#),
        "{}",
        answered.sent
    );
    for control in [
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
    ] {
        assert!(
            !answered.sent.contains(control),
            "{control} leaked: {}",
            answered.sent
        );
    }
    assert!(
        answered
            .sent
            .to_ascii_lowercase()
            .contains("authorization: bearer oracle-key"),
        "{}",
        answered.sent
    );
    assert!(
        answered
            .sent
            .to_ascii_lowercase()
            .contains("content-type: application/json"),
        "{}",
        answered.sent
    );
    assert!(
        answered.sent.to_ascii_lowercase().contains("x-caller: yes"),
        "{}",
        answered.sent
    );
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

async fn stream_not_built() {
    let body = r#"{"model":"text-model","prompt":"hi","stream":true}"#;
    let answered = call("/v1/completions", body, 200, ANSWER).await;
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

    // LiteLLM never parses this body, so it has no call id to put in the error body
    let malformed = call_with_call_id_in_body("{", true).await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert!(
        !malformed.raw.contains("litellm_call_id"),
        "{}",
        malformed.raw
    );

    // an empty body counts as {}, the way LiteLLM reads it, so its refusal keeps the id
    let empty = call_with_call_id_in_body("", true).await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert!(
        empty.raw.contains(r#""litellm_call_id":"caller-id""#),
        "{}",
        empty.raw
    );

    // a body that parses as JSON but not as a completion request is read before LiteLLM builds the call id, so the id stays
    let data = call_with_call_id_in_body(
        r#"{"model":"text-model","prompt":"hi","stream":"yes"}"#,
        true,
    )
    .await;
    assert_eq!(data.status, StatusCode::BAD_REQUEST);
    assert!(
        data.raw.contains(r#""litellm_call_id":"caller-id""#),
        "{}",
        data.raw
    );

    // this body parsed, so its refusal carries the id LiteLLM had already resolved
    let refused = call_with_call_id_in_body(
        r#"{"model":"text-model","prompt":"hi","fallbacks":["b"]}"#,
        true,
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(
        refused.raw.contains(r#""litellm_call_id":"caller-id""#),
        "{}",
        refused.raw
    );
}

async fn auth_required() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let source = format!(
        "general_settings: {{master_key: oracle-master}}\nmodel_list:\n  - model_name: text-model\n    litellm_params: {{model: openai/gpt-3.5-turbo-instruct, api_base: 'http://{address}/v1', api_key: oracle-key}}\n"
    );
    let state = crate::routes::tests::state_for(&config(&source));
    let application = || {
        crate::routes::router(
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
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
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
