#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use rstest::rstest;
use serde_json::Value;
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

struct Answered {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
    sent: String,
}

async fn call(path: &str, body: &str, upstream_status: u16, upstream: &str) -> Answered {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let state = state(&format!("http://{address}/v1"));
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer oracle-master")
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
    Answered {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
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
        Scenario::ProviderBadRequest => provider_error(400, "invalid_request_error").await,
        Scenario::ProviderRateLimit => provider_error(429, "rate_limit_error").await,
        Scenario::StreamNotBuilt => stream_not_built().await,
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
        answered.body.pointer("/choices/0/text"),
        Some(&Value::String(String::from(" hello")))
    );
    assert_eq!(
        answered.body.pointer("/object"),
        Some(&Value::String(String::from("text_completion")))
    );
    assert!(
        answered.sent.starts_with("POST /v1/completions "),
        "{}",
        answered.sent
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
    assert_eq!(answered.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        answered.body.pointer("/error/type"),
        Some(&Value::String(String::from("internal_server_error")))
    );
    assert_eq!(
        answered.body.pointer("/error/code"),
        Some(&Value::String(String::from("500")))
    );
    assert!(answered.sent.is_empty(), "no provider request");
}

async fn mixed_prompt_rejected() {
    let answered = text_answer("/v1/completions", r#"[["a"]]"#).await;
    assert_eq!(answered.status, StatusCode::INTERNAL_SERVER_ERROR);
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
}

async fn parameters_and_controls() {
    let body = r#"{"model":"text-model","prompt":"hi","temperature":0.7,"logprobs":2,"max_retries":5,"api_base":"http://evil.example","api_key":"sk-evil","metadata":{"user":"x"},"num_retries":3,"timeout":1}"#;
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
    for control in [
        "max_retries",
        "api_base",
        "api_key",
        "metadata",
        "num_retries",
        "sk-evil",
        "evil.example",
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
}

async fn provider_error(status: u16, error_type: &str) {
    let upstream = r#"{"error":{"message":"provider refused","type":"invalid_request_error","param":"prompt","code":"bad"}}"#;
    let body = r#"{"model":"text-model","prompt":"hi"}"#;
    let answered = call("/v1/completions", body, status, upstream).await;
    assert_eq!(answered.status.as_u16(), status);
    assert_eq!(
        answered.body.pointer("/error/type"),
        Some(&Value::String(error_type.to_owned()))
    );
    assert_eq!(
        answered.body.pointer("/error/code"),
        Some(&Value::String(status.to_string()))
    );
    assert_eq!(
        answered
            .headers
            .get("llm_provider-x-request-id")
            .map(|value| value.to_str().ok()),
        Some(Some("provider-call"))
    );
}

async fn stream_not_built() {
    let body = r#"{"model":"text-model","prompt":"hi","stream":true}"#;
    let answered = call("/v1/completions", body, 200, ANSWER).await;
    assert_eq!(answered.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        answered.body.pointer("/error/type"),
        Some(&Value::String(String::from("internal_server_error")))
    );
    assert!(answered.sent.is_empty(), "no provider request");
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
    let authorized = application()
        .oneshot(
            Request::post("/v1/completions")
                .header("content-type", "application/json")
                .header("authorization", "Bearer oracle-master")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");
    let (response, sent) = tokio::join!(authorized, serve(listener, 200, ANSWER));
    assert_eq!(response.expect("response").status(), StatusCode::OK);
    assert!(sent.starts_with("POST /v1/completions "), "{sent}");
}
