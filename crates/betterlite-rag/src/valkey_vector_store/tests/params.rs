use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::ExposeSecret;
use serde_json::Number;

use super::fakes::{empty_params, params};
use crate::valkey_vector_store::error::ValkeyError;
use crate::valkey_vector_store::params::{DEFAULT_PORT, ValkeyParams};

#[test]
fn params_read_defaults() {
    let view = ValkeyParams::read(&empty_params()).expect("the default view");
    assert_eq!(view.port, DEFAULT_PORT);
    assert!(!view.ssl);
    assert_eq!(view.text_field, "text");
    assert_eq!(view.embedding_field, "embedding");
    assert!(view.host.is_none());
    assert!(view.embedding_model.is_none());
    assert!(view.embedding_config.is_empty());
    assert!(view.password.is_none());
}

#[test]
fn params_read_every_key() {
    let source = r#"{
        "litellm_embedding_model": "openai/text-embedding-3-small",
        "litellm_embedding_config": {"dimensions": 8},
        "valkey_host": "valkey.example.com",
        "valkey_port": 6380,
        "valkey_password": "p@ss",
        "valkey_ssl": true,
        "valkey_text_field": "content",
        "valkey_embedding_field": "vector",
        "unrelated": {"ignored": true}
    }"#;
    let view = ValkeyParams::read(&params(source)).expect("the explicit view");
    assert_eq!(
        view.embedding_model.as_deref(),
        Some("openai/text-embedding-3-small")
    );
    assert_eq!(
        view.embedding_config.get("dimensions"),
        Some(&Json::Number(Number::from(8_u64)))
    );
    assert_eq!(view.host.as_deref(), Some("valkey.example.com"));
    assert_eq!(view.port, 6380);
    assert_eq!(
        view.password.as_ref().map(ExposeSecret::expose_secret),
        Some("p@ss")
    );
    assert!(view.ssl);
    assert_eq!(view.text_field, "content");
    assert_eq!(view.embedding_field, "vector");
}

#[test]
fn params_null_values_take_the_defaults() {
    let source = r#"{"valkey_port":null,"valkey_ssl":null,"valkey_text_field":null}"#;
    let view = ValkeyParams::read(&params(source)).expect("the null view");
    assert_eq!(view.port, DEFAULT_PORT);
    assert!(!view.ssl);
    assert_eq!(view.text_field, "text");
}

#[rstest]
#[case::embedding_model(
    r#"{"litellm_embedding_model":7}"#,
    "litellm_embedding_model",
    "a string"
)]
#[case::embedding_config(
    r#"{"litellm_embedding_config":[]}"#,
    "litellm_embedding_config",
    "an object"
)]
#[case::host(r#"{"valkey_host":{}}"#, "valkey_host", "a string")]
#[case::port(r#"{"valkey_port":"6379"}"#, "valkey_port", "an integer")]
#[case::port_fraction(r#"{"valkey_port":6379.5}"#, "valkey_port", "an integer")]
#[case::password(r#"{"valkey_password":true}"#, "valkey_password", "a string")]
#[case::ssl(r#"{"valkey_ssl":"true"}"#, "valkey_ssl", "a boolean")]
#[case::text_field(r#"{"valkey_text_field":3}"#, "valkey_text_field", "a string")]
#[case::embedding_field(
    r#"{"valkey_embedding_field":[]}"#,
    "valkey_embedding_field",
    "a string"
)]
fn params_reject_wrong_types(#[case] source: &str, #[case] field: &str, #[case] expected: &str) {
    let error = ValkeyParams::read(&params(source))
        .err()
        .expect("the wrong type");
    assert_eq!(
        error.to_string(),
        format!("the {field} value has the wrong type: expected {expected}")
    );
}

#[test]
fn params_reject_a_port_out_of_range() {
    let error = ValkeyParams::read(&params(r#"{"valkey_port":70000}"#))
        .err()
        .expect("the port");
    assert_eq!(
        error.to_string(),
        "valkey_port must be between 0 and 65535, got 70000"
    );
}

#[test]
fn params_connection_url_encodes_the_password_and_scheme() {
    let source = r#"{"valkey_host":"valkey.example.com","valkey_port":6380,"valkey_password":"p@ss","valkey_ssl":true}"#;
    let view = ValkeyParams::read(&params(source)).expect("the view");
    assert_eq!(
        view.connection_url().expect("the url").expose_secret(),
        "rediss://:p%40ss@valkey.example.com:6380"
    );
}

#[test]
fn params_connection_url_requires_a_host() {
    let view = ValkeyParams::read(&empty_params()).expect("the view");
    let error = view.connection_url().expect_err("the missing host");
    assert_eq!(
        error.to_string(),
        "valkey_host is required in litellm_params for the Valkey vector store. Set it on the vector store's litellm_params, e.g. valkey_host: my-valkey.example.com"
    );
}

#[test]
fn params_connection_url_keeps_an_empty_host_as_given() {
    let view = ValkeyParams::read(&params(r#"{"valkey_host":""}"#)).expect("the view");
    assert_eq!(
        view.connection_url().expect("the url").expose_secret(),
        "redis://:6379"
    );
}

#[test]
fn params_reports_the_missing_embedding_model_message() {
    let view = ValkeyParams::read(&empty_params()).expect("the view");
    assert!(view.embedding_model.is_none());
    let message = ValkeyError::EmbeddingModelRequired.to_string();
    assert_eq!(
        message,
        "litellm_embedding_model is required in litellm_params for the Valkey vector store. Example: litellm_params['litellm_embedding_model'] = 'openai/text-embedding-3-small'"
    );
}
