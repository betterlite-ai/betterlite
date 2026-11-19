#![cfg(test)]

use std::future::{Future, ready};

use betterlite_config::sections::copilot_chat::Settings;
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::EmbeddingConfig;
use super::error::CopilotEmbeddingError;
use super::reply::{get_error_class, transform_embedding_response};
use super::request::{SUPPORTED_OPENAI_PARAMS, map_openai_params, transform_embedding_request};
use crate::api::copilot_chat::credentials::CopilotCredentials;

const REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,-1]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}"#;

#[derive(Clone, Copy)]
enum Outcome {
    Key(&'static str),
    Empty,
    NoToken,
    Refresh,
}

struct Stored {
    outcome: Outcome,
    base: Option<&'static str>,
}

impl CopilotCredentials for Stored {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        ready(self.base.map(String::from))
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        ready(match self.outcome {
            Outcome::Key(text) => Ok(Some(SecretString::from(String::from(text)))),
            Outcome::Empty => Ok(Some(SecretString::from(String::new()))),
            Outcome::NoToken => Ok(None),
            Outcome::Refresh => Err(ApiKeyError::Refresh),
        })
    }
}

fn config(outcome: Outcome, base: Option<&'static str>) -> EmbeddingConfig<Stored> {
    EmbeddingConfig {
        credentials: Stored { outcome, base },
        settings: Settings::default(),
    }
}

#[test]
fn advertised_params_are_the_four_embedding_parameters() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["timeout", "dimensions", "encoding_format", "user"]
    );
}

#[test]
fn map_params_copies_only_the_advertised_parameters() {
    let requested = Object::from_iter([
        (String::from("timeout"), Json::Number(Number::from(30_u32))),
        (
            String::from("dimensions"),
            Json::Number(Number::from(8_u32)),
        ),
        (
            String::from("encoding_format"),
            Json::Text(String::from("float")),
        ),
        (String::from("user"), Json::Text(String::from("u"))),
        (String::from("model"), Json::Text(String::from("m2"))),
        (String::from("input"), Json::Text(String::from("x"))),
        (String::from("unknown"), Json::Number(Number::from(1_u32))),
    ]);
    let mut optional =
        Object::from_iter([(String::from("keep"), Json::Number(Number::from(1_u32)))]);

    map_openai_params(&requested, &mut optional);

    assert_eq!(
        optional,
        Object::from_iter([
            (String::from("keep"), Json::Number(Number::from(1_u32))),
            (String::from("timeout"), Json::Number(Number::from(30_u32))),
            (
                String::from("dimensions"),
                Json::Number(Number::from(8_u32))
            ),
            (
                String::from("encoding_format"),
                Json::Text(String::from("float")),
            ),
            (String::from("user"), Json::Text(String::from("u"))),
        ])
    );
}

#[rstest]
#[case::scalar(
    EmbeddingInput::Text(String::from("hello")),
    Json::List(vec![Json::Text(String::from("hello"))])
)]
#[case::texts(
    EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]),
    Json::List(vec![Json::Text(String::from("a")), Json::Text(String::from("b"))])
)]
#[case::tokens(
    EmbeddingInput::Tokens(vec![1, 2]),
    Json::List(vec![Json::Number(Number::from(1_u32)), Json::Number(Number::from(2_u32))])
)]
#[case::token_lists(
    EmbeddingInput::TokenLists(vec![vec![1], vec![2, 3]]),
    Json::List(vec![
        Json::List(vec![Json::Number(Number::from(1_u32))]),
        Json::List(vec![Json::Number(Number::from(2_u32)), Json::Number(Number::from(3_u32))]),
    ])
)]
fn transform_converts_every_input_shape(#[case] input: EmbeddingInput, #[case] expected: Json) {
    let body = transform_embedding_request("text-embedding-3-small", &input, &Object::new());

    assert_eq!(body.get("input"), Some(&expected));
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("text-embedding-3-small")))
    );
}

#[test]
fn transform_removes_one_leading_provider_prefix_only() {
    let body = transform_embedding_request(
        "github_copilot/github_copilot/text-embedding-3-small",
        &EmbeddingInput::Text(String::from("hello")),
        &Object::new(),
    );

    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from(
            "github_copilot/text-embedding-3-small"
        )))
    );
}

#[test]
fn transform_lets_optional_params_replace_model_and_input() {
    let optional = Object::from_iter([
        (String::from("model"), Json::Text(String::from("override"))),
        (
            String::from("input"),
            Json::List(vec![Json::Text(String::from("given"))]),
        ),
        (
            String::from("dimensions"),
            Json::Number(Number::from(256_u32)),
        ),
    ]);

    let body = transform_embedding_request(
        "github_copilot/text-embedding-3-small",
        &EmbeddingInput::Text(String::from("hello")),
        &optional,
    );

    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("override")))
    );
    assert_eq!(
        body.get("input"),
        Some(&Json::List(vec![Json::Text(String::from("given"))]))
    );
    assert_eq!(
        body.get("dimensions"),
        Some(&Json::Number(Number::from(256_u32)))
    );
}

#[tokio::test]
async fn validate_environment_uses_the_stored_key_and_lets_caller_headers_win() {
    let mut config = config(Outcome::Key("stored-key"), None);
    let caller = Headers::from_iter([
        (
            String::from("editor-version"),
            String::from("caller-editor"),
        ),
        (String::from("X-Initiator"), String::from("caller")),
    ]);

    let headers = config
        .validate_environment(caller, "gpt-4o")
        .await
        .expect("the stored key validates");

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer stored-key")
    );
    assert_eq!(
        headers.get("editor-version").map(String::as_str),
        Some("caller-editor")
    );
    assert_eq!(
        headers.get("X-Initiator").map(String::as_str),
        Some("caller")
    );
    assert_eq!(
        headers.get("copilot-integration-id").map(String::as_str),
        Some("vscode-chat")
    );
    assert_eq!(
        headers.get("x-github-api-version").map(String::as_str),
        Some("2025-04-01")
    );
    assert!(headers.contains_key("x-request-id"));
    assert_eq!(headers.len(), 11);
}

#[rstest]
#[case::no_token(Outcome::NoToken)]
#[case::empty_token(Outcome::Empty)]
#[tokio::test]
async fn validate_environment_rejects_a_falsey_key(#[case] outcome: Outcome) {
    let mut config = config(outcome, None);

    let error = config
        .validate_environment(Headers::new(), "gpt-4o")
        .await
        .expect_err("a falsey key is an authentication error");

    assert!(matches!(&error, CopilotEmbeddingError::MissingKey { .. }));
    assert_eq!(error.status(), Some(401));
    assert_eq!(
        error.to_string(),
        "GitHub Copilot API key is required. Please authenticate via OAuth Device Flow."
    );
}

#[tokio::test]
async fn validate_environment_translates_a_key_error() {
    let mut config = config(Outcome::Refresh, None);

    let error = config
        .validate_environment(Headers::new(), "gpt-4o")
        .await
        .expect_err("a key error is an authentication error");

    assert!(matches!(
        &error,
        CopilotEmbeddingError::Authentication { .. }
    ));
    assert_eq!(error.status(), Some(401));
}

#[rstest]
#[case::explicit(
    Some("https://cached.test"),
    None,
    Some("https://explicit.test//"),
    "https://explicit.test/embeddings"
)]
#[case::cached_above_env(
    Some("https://cached.test/"),
    Some("https://env.test"),
    None,
    "https://cached.test/embeddings"
)]
#[case::env_above_default(None, Some("https://env.test///"), None, "https://env.test/embeddings")]
#[case::default(None, None, None, "https://api.githubcopilot.com/embeddings")]
#[tokio::test]
async fn url_precedence_is_explicit_then_cached_then_environment_then_default(
    #[case] cached: Option<&'static str>,
    #[case] env_base: Option<&str>,
    #[case] explicit: Option<&str>,
    #[case] expected: &str,
) {
    let mut config = EmbeddingConfig {
        credentials: Stored {
            outcome: Outcome::Key("stored-key"),
            base: cached,
        },
        settings: Settings {
            disable_system_to_assistant: false,
            api_base: env_base.map(str::to_owned),
        },
    };

    assert_eq!(config.get_complete_url(explicit).await, expected);
}

#[test]
fn response_logging_sees_the_raw_text_before_the_reply_decodes() {
    let mut seen = String::new();

    let response = transform_embedding_response(REPLY.as_bytes(), &mut |text: &str| {
        text.clone_into(&mut seen);
    })
    .expect("the reply decodes");

    assert_eq!(seen, REPLY);
    assert_eq!(response.model, "text-embedding-3-small");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.usage.total_tokens, 2);
}

#[test]
fn response_decoding_fails_after_logging_the_raw_text() {
    let mut logged = 0_usize;

    let error = transform_embedding_response(b"not json", &mut |_text| logged += 1)
        .expect_err("a non-JSON reply does not decode");

    assert_eq!(logged, 1);
    assert!(matches!(error, CopilotEmbeddingError::Decode { .. }));
    assert_eq!(error.status(), None);
}

#[test]
fn error_class_is_the_openai_error_with_the_supplied_status_and_headers() {
    let headers = Headers::from_iter([(String::from("x-caller"), String::from("v"))]);

    let error = CopilotEmbeddingError::from(get_error_class("boom", 429, &headers));

    assert_eq!(error.status(), Some(429));
    let CopilotEmbeddingError::OpenAi(OpenAiError::Provider(provider)) = &error else {
        panic!("a provider error is expected");
    };
    assert_eq!(provider.status_code, 429);
    assert_eq!(provider.message, "boom");
    assert_eq!(
        provider.headers,
        Some(vec![(String::from("x-caller"), String::from("v"))])
    );
    assert!(!provider.status_code_is_synthesized);
}
