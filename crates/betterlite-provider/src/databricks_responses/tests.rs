#![cfg(test)]

use crate::databricks_auth::Headers;
use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::error::{DatabricksError, SDK_API_BASE, SDK_CREDENTIALS};
use crate::databricks_auth::oauth::{OauthReply, OauthTokenRequest, OauthTokenSource};
use crate::databricks_responses::{DatabricksResponsesAPIConfig, ResponsesHeaders};
use async_trait::async_trait;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

const VERSION: &str = "1.105.1";

struct NoTokens;

#[async_trait]
impl OauthTokenSource for NoTokens {
    async fn send(&self, _request: &OauthTokenRequest) -> OauthReply {
        OauthReply::Transport("the token source is not expected here".to_owned())
    }
}

fn env() -> DatabricksEnv {
    DatabricksEnv {
        client_id: None,
        client_secret: None,
        api_base: None,
        user_agent_override: None,
        version: VERSION.to_owned(),
    }
}

fn key() -> SecretString {
    SecretString::from("pat".to_owned())
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the runtime builds")
}

fn resolve_headers(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
) -> Result<Headers, DatabricksError> {
    let runtime = runtime();
    runtime.block_on(DatabricksResponsesAPIConfig.headers(
        ResponsesHeaders {
            api_key,
            api_base,
            headers: None,
        },
        &env(),
        &NoTokens,
    ))
}

fn input(source: &str) -> ResponsesInput {
    serde_json::from_str(source).expect("the input parses")
}

#[test]
fn the_config_names_databricks_and_refuses_a_native_websocket() {
    let config = DatabricksResponsesAPIConfig;
    assert_eq!(config.provider(), ProviderId::Databricks);
    assert!(!config.supports_native_websocket());
}

#[test]
fn headers_carry_the_key_base_and_content_type() {
    let pat = key();
    let headers = resolve_headers(Some(&pat), Some("https://host")).expect("a key resolves");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer pat")
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("User-Agent").map(String::as_str),
        Some("litellm/1.105.1")
    );
}

#[test]
fn headers_fall_back_to_the_env_base() {
    let mut env = env();
    env.api_base = Some("https://env-host".to_owned());
    let pat = key();
    let runtime = runtime();
    let headers = runtime
        .block_on(DatabricksResponsesAPIConfig.headers(
            ResponsesHeaders {
                api_key: Some(&pat),
                api_base: None,
                headers: None,
            },
            &env,
            &NoTokens,
        ))
        .expect("the env base resolves");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer pat")
    );
}

#[test]
fn a_missing_key_reaches_the_sdk_error() {
    let error = resolve_headers(None, Some("https://host")).expect_err("no key");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, SDK_CREDENTIALS);
}

#[test]
fn the_responses_url_appends_the_suffix() {
    let config = DatabricksResponsesAPIConfig;
    assert_eq!(
        config
            .complete_url(Some("https://host/"), &env())
            .expect("the base is present"),
        "https://host/responses"
    );
}

#[test]
fn the_responses_url_uses_the_env_base() {
    let mut env = env();
    env.api_base = Some("https://env-host".to_owned());
    let config = DatabricksResponsesAPIConfig;
    assert_eq!(
        config
            .complete_url(None, &env)
            .expect("the env base is present"),
        "https://env-host/responses"
    );
}

#[test]
fn a_missing_responses_base_is_the_sdk_error() {
    let error = DatabricksResponsesAPIConfig
        .complete_url(None, &env())
        .expect_err("no base");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, SDK_API_BASE);
}

#[test]
fn the_request_strips_the_databricks_prefix() {
    let body = DatabricksResponsesAPIConfig
        .create_request(
            "databricks/databricks-gpt-5",
            &input(r#""hi""#),
            &Object::new(),
            None,
        )
        .expect("the request transforms");
    assert_eq!(
        body.get("model"),
        Some(&Json::Text("databricks-gpt-5".to_owned()))
    );
}

#[test]
fn a_foreign_function_call_id_survives_for_databricks() {
    let body = DatabricksResponsesAPIConfig
        .create_request(
            "gpt",
            &input(r#"[{"type":"function_call","id":"call_1","call_id":"c1","name":"f","arguments":"{}"}]"#),
            &Object::new(),
            None,
        )
        .expect("the request transforms");
    let rendered = serde_json::to_string(&body).expect("the body renders");
    assert!(rendered.contains("call_1"));
}

#[rstest]
#[case::plain("gpt", "gpt")]
#[case::one_dot("gpt.5", "gpt.5")]
fn a_model_without_the_prefix_is_kept(#[case] model: &str, #[case] expected: &str) {
    let body = DatabricksResponsesAPIConfig
        .create_request(model, &input(r#""hi""#), &Object::new(), None)
        .expect("the request transforms");
    assert_eq!(body.get("model"), Some(&Json::Text(expected.to_owned())));
}
