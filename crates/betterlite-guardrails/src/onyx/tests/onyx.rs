use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::onyx::{
    DEFAULT_API_BASE, DEFAULT_TIMEOUT, OnyxGuardrail, OnyxGuardrailConfigModel, OnyxGuardrailError,
    SUPPORTED_EVENT_HOOKS,
};
use betterlite_http::proxy::base_builder;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::support::{
    captured, captured_path, guardrail, guardrail_with_timeout, inputs, mock_onyx, object,
    silent_onyx, value,
};

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn build(
    key: Option<SecretString>,
    base: Option<String>,
    timeout: Option<f64>,
    environment: &BTreeMap<String, String>,
) -> Result<OnyxGuardrail, OnyxGuardrailError> {
    OnyxGuardrail::new(key, base, timeout, environment, base_builder())
}

#[test]
fn environment_reads_api_key_api_base_and_timeout_from_the_environment() {
    let environment = lookup(&[
        ("ONYX_API_KEY", "env-key"),
        ("ONYX_API_BASE", "https://guard.example"),
        ("ONYX_TIMEOUT", "7"),
    ]);
    let guardrail = build(None, None, None, &environment).unwrap();
    assert_eq!(guardrail.api_key.expose_secret(), "env-key");
    assert_eq!(guardrail.api_base, "https://guard.example");
    assert_eq!(guardrail.timeout, Duration::from_secs(7));
}

#[test]
fn environment_defaults_api_base_and_timeout_when_unset() {
    let guardrail = build(None, None, None, &lookup(&[("ONYX_API_KEY", "env-key")])).unwrap();
    assert_eq!(guardrail.api_base, DEFAULT_API_BASE);
    assert_eq!(guardrail.timeout, DEFAULT_TIMEOUT);
}

#[test]
fn environment_prefers_configured_values_over_the_environment() {
    let environment = lookup(&[
        ("ONYX_API_KEY", "env-key"),
        ("ONYX_API_BASE", "https://env.example"),
        ("ONYX_TIMEOUT", "7"),
    ]);
    let guardrail = build(
        Some(SecretString::from("configured-key".to_owned())),
        Some("https://configured.example".to_owned()),
        Some(2.5_f64),
        &environment,
    )
    .unwrap();
    assert_eq!(guardrail.api_key.expose_secret(), "configured-key");
    assert_eq!(guardrail.api_base, "https://configured.example");
    assert_eq!(guardrail.timeout, Duration::from_millis(2_500));
}

#[test]
fn environment_rejects_a_missing_api_key() {
    let error = build(None, None, None, &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, OnyxGuardrailError::MissingApiKey));
}

#[test]
fn environment_treats_an_empty_timeout_as_unset() {
    let environment = lookup(&[("ONYX_API_KEY", "env-key"), ("ONYX_TIMEOUT", "")]);
    let guardrail = build(None, None, None, &environment).unwrap();
    assert_eq!(guardrail.timeout, DEFAULT_TIMEOUT);
}

#[test]
fn timeout_reads_trimmed_integer_seconds() {
    let environment = lookup(&[("ONYX_API_KEY", "env-key"), ("ONYX_TIMEOUT", " 7 ")]);
    let guardrail = build(None, None, None, &environment).unwrap();
    assert_eq!(guardrail.timeout, Duration::from_secs(7));
}

#[test]
fn timeout_rejects_a_fractional_environment_value() {
    let environment = lookup(&[("ONYX_API_KEY", "env-key"), ("ONYX_TIMEOUT", "7.5")]);
    let error = build(None, None, None, &environment).unwrap_err();
    assert!(matches!(
        error,
        OnyxGuardrailError::InvalidTimeout { value } if value == "7.5"
    ));
}

#[test]
fn timeout_rejects_a_negative_environment_value() {
    let environment = lookup(&[("ONYX_API_KEY", "env-key"), ("ONYX_TIMEOUT", "-7")]);
    let error = build(None, None, None, &environment).unwrap_err();
    assert!(matches!(
        error,
        OnyxGuardrailError::InvalidTimeout { value } if value == "-7"
    ));
}

#[test]
fn timeout_rejects_a_configured_value_that_is_not_a_duration() {
    let error = build(
        Some(SecretString::from("key".to_owned())),
        None,
        Some(f64::NAN),
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        OnyxGuardrailError::UnrepresentableTimeout { .. }
    ));
}

#[test]
fn config_model_defaults_every_field_to_none() {
    let model = OnyxGuardrailConfigModel::default();
    assert!(model.api_base.is_none());
    assert!(model.api_key.is_none());
    assert!(model.timeout.is_none());
}

#[test]
fn config_model_deserializes_api_base_api_key_and_timeout() {
    let model: OnyxGuardrailConfigModel = serde_json::from_str(
        r#"{"api_base": "https://guard.example", "api_key": "key", "timeout": 2.5}"#,
    )
    .unwrap();
    assert_eq!(model.api_base.as_deref(), Some("https://guard.example"));
    assert_eq!(model.api_key.as_deref(), Some("key"));
    assert_eq!(model.timeout, Some(2.5_f64));
}

#[test]
fn config_model_ui_friendly_name_is_onyx_guardrail() {
    assert_eq!(
        OnyxGuardrailConfigModel::ui_friendly_name(),
        "Onyx Guardrail"
    );
}

#[test]
fn event_hooks_are_pre_call_during_call_and_post_call() {
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).unwrap(),
        r#"["pre_call","during_call","post_call"]"#
    );
}

#[tokio::test]
async fn request_payload_posts_the_proxy_server_request_to_the_keyed_endpoint() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    let request_data = object(
        r#"{"proxy_server_request": {"url": "http://host/v1/chat", "body": {"model": "m"}}}"#,
    );
    let returned = guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &request_data,
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap();
    let captured = captured(&mock);
    assert_eq!(
        captured_path(&mock),
        "/guard/evaluate/v1/oracle-key/litellm"
    );
    assert_eq!(
        captured.payload,
        value(r#"{"url": "http://host/v1/chat", "body": {"model": "m"}}"#)
    );
    assert_eq!(captured.input_type, GuardrailInputType::Request);
    assert_eq!(captured.conversation_id, "call-1");
    assert_eq!(returned, inputs(r#"{"texts": ["hello"]}"#));
}

#[tokio::test]
async fn request_payload_defaults_to_an_empty_object() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object(r#"{"model": "m"}"#),
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap();
    assert_eq!(captured(&mock).payload, value("{}"));
}

#[tokio::test]
async fn response_payload_posts_the_response_object() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    let request_data = object(
        r#"{"model": "gpt-4o", "response": {"id": "chatcmpl-1", "object": "chat.completion",
            "created": 1700000000, "model": "gpt-4o",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}]}}"#,
    );
    guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["done"]}"#),
            &request_data,
            GuardrailInputType::Response,
            Some("call-1"),
        )
        .await
        .unwrap();
    let captured = captured(&mock);
    assert_eq!(captured.input_type, GuardrailInputType::Response);
    assert_eq!(
        captured.payload,
        value(
            r#"{"id": "chatcmpl-1", "object": "chat.completion", "created": 1700000000,
                "model": "gpt-4o",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}]}"#
        )
    );
}

#[tokio::test]
async fn response_payload_defaults_to_an_empty_object_without_a_response() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["done"]}"#),
            &object(r#"{"model": "gpt-4o"}"#),
            GuardrailInputType::Response,
            Some("call-1"),
        )
        .await
        .unwrap();
    assert_eq!(captured(&mock).payload, value("{}"));
}

#[tokio::test]
async fn response_payload_falls_back_to_the_request_data_when_model_response_rejects_it() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    let request_data = object(r#"{"id": 123, "response": {"some": "thing"}}"#);
    guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["done"]}"#),
            &request_data,
            GuardrailInputType::Response,
            Some("call-1"),
        )
        .await
        .unwrap();
    assert_eq!(
        captured(&mock).payload,
        value(r#"{"id": 123, "response": {"some": "thing"}}"#)
    );
}

#[tokio::test]
async fn conversation_id_uses_the_logging_call_id() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object("{}"),
            GuardrailInputType::Request,
            Some("call-42"),
        )
        .await
        .unwrap();
    assert_eq!(captured(&mock).conversation_id, "call-42");
}

#[tokio::test]
async fn conversation_id_generates_a_uuid_without_a_logging_call_id() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object("{}"),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        captured(&mock).conversation_id,
        "00000000-0000-0000-0000-000000000001"
    );
}

#[tokio::test]
async fn allowed_response_returns_the_inputs_unchanged() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": true}"#)).await;
    let returned = guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"], "model": "gpt-4o"}"#),
            &object("{}"),
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap();
    assert_eq!(
        returned,
        inputs(r#"{"texts": ["hello"], "model": "gpt-4o"}"#)
    );
}

#[tokio::test]
async fn a_missing_allowed_field_is_allowed() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"other": 1}"#)).await;
    guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object("{}"),
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn block_raises_a_400_with_the_violated_rules_message() {
    let mock = mock_onyx(
        StatusCode::OK,
        String::from(r#"{"allowed": false, "violated_rules": ["pii", "jailbreak"]}"#),
    )
    .await;
    let error = guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object("{}"),
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Request blocked by Onyx Guard. Violations: pii, jailbreak."
    );
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn block_without_violated_rules_reports_unknown_violation() {
    let mock = mock_onyx(StatusCode::OK, String::from(r#"{"allowed": false}"#)).await;
    let error = guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object("{}"),
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Request blocked by Onyx Guard. Violations: Unknown violation."
    );
}

#[tokio::test]
async fn fail_open_returns_the_inputs_on_a_backend_status_error() {
    let mock = mock_onyx(
        StatusCode::INTERNAL_SERVER_ERROR,
        String::from(r#"{"detail": "down"}"#),
    )
    .await;
    let returned = guardrail(&mock.base)
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object("{}"),
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap();
    assert_eq!(returned, inputs(r#"{"texts": ["hello"]}"#));
}

#[tokio::test]
async fn fail_open_returns_the_inputs_on_a_timeout() {
    let mock = silent_onyx().await;
    let returned = guardrail_with_timeout(&mock.base, Duration::from_millis(100))
        .apply_guardrail(
            inputs(r#"{"texts": ["hello"]}"#),
            &object("{}"),
            GuardrailInputType::Request,
            Some("call-1"),
        )
        .await
        .unwrap();
    assert_eq!(returned, inputs(r#"{"texts": ["hello"]}"#));
}
