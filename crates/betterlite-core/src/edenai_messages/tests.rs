#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json as HiddenJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::Constraints;
use betterlite_openai::edenai_common::EdenAiEnv;
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Number, Value};

use super::{
    API_BASE_ENV, API_KEY_ENV, BASE_CLASS, EdenAiMessagesError, EnvironmentInput, ReplyInput,
    descriptor, transform_response, validate_environment,
};
use crate::api::anthropic_beta_filter::Headers;
use crate::api::api_boundaries::ApiLimits;
use crate::json_messages_policy::{JsonMessagesPolicy, MessagesCredentials};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

fn state() -> LoggingState {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap()
}

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn policy() -> JsonMessagesPolicy {
    JsonMessagesPolicy {
        descriptor: descriptor(),
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn descriptor_carries_the_eden_slug_and_environment_names() {
    let descriptor = descriptor();
    assert_eq!(descriptor.provider, ProviderId::Edenai);
    assert_eq!(descriptor.config.base_url, "https://api.edenai.run/v3");
    assert_eq!(descriptor.config.api_key_env, API_KEY_ENV);
    assert_eq!(
        descriptor.config.api_base_env.as_deref(),
        Some(API_BASE_ENV)
    );
    assert_eq!(descriptor.config.base_class.0, BASE_CLASS);
    assert_eq!(descriptor.config.constraints, Constraints::default());
    assert!(descriptor.config.param_mappings.is_empty());
    assert!(descriptor.config.special_handling.is_empty());
    assert!(descriptor.config.supported_endpoints.is_empty());
}

#[test]
fn validate_environment_adds_the_bearer_and_anthropic_headers() {
    let environment = EdenAiEnv::default();
    let lookup = lookup(&[]);
    let optional = Object::new();
    let headers = validate_environment(
        &policy(),
        EnvironmentInput {
            headers: Headers::new(),
            api_key: Some(&key("eden")),
            model: "m",
            env: &environment,
            lookup: &lookup,
            optional_params: &optional,
            messages: &[],
        },
    )
    .unwrap();
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer eden")
    );
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn validate_environment_without_a_key_is_the_missing_key_authentication_error() {
    let environment = EdenAiEnv::default();
    let lookup = lookup(&[]);
    let optional = Object::new();
    let error = validate_environment(
        &policy(),
        EnvironmentInput {
            headers: Headers::new(),
            api_key: None,
            model: "m",
            env: &environment,
            lookup: &lookup,
            optional_params: &optional,
            messages: &[],
        },
    )
    .unwrap_err();
    assert!(matches!(&error, EdenAiMessagesError::MissingKey(_)));
    assert_eq!(error.status_code(), Some(401));
    assert_eq!(error.class_name(), "AuthenticationError");
    assert_eq!(
        error.to_string(),
        "Missing Eden AI API key: set EDENAI_API_KEY or pass api_key"
    );
}

#[rstest]
#[case::explicit(Some("explicit"), Some("global"), Some("environment"), "explicit")]
#[case::global(None, Some("global"), Some("environment"), "global")]
#[case::environment(None, None, Some("environment"), "environment")]
fn validate_environment_reads_the_explicit_then_the_global_then_the_environment_key(
    #[case] explicit: Option<&str>,
    #[case] global: Option<&str>,
    #[case] environment_key: Option<&str>,
    #[case] expected: &str,
) {
    let environment = EdenAiEnv {
        api_key: environment_key.map(key),
        api_base: None,
        global_api_key: global.map(key),
    };
    let lookup = lookup(&[]);
    let optional = Object::new();
    let chosen = explicit.map(key);
    let headers = validate_environment(
        &policy(),
        EnvironmentInput {
            headers: Headers::new(),
            api_key: chosen.as_ref(),
            model: "m",
            env: &environment,
            lookup: &lookup,
            optional_params: &optional,
            messages: &[],
        },
    )
    .unwrap();
    let bearer = format!("Bearer {expected}");
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some(bearer.as_str())
    );
}

#[test]
fn validate_environment_keeps_a_callers_x_api_key() {
    let environment = EdenAiEnv::default();
    let lookup = lookup(&[]);
    let optional = Object::new();
    let headers = validate_environment(
        &policy(),
        EnvironmentInput {
            headers: headers(&[("x-api-key", "caller")]),
            api_key: Some(&key("eden")),
            model: "m",
            env: &environment,
            lookup: &lookup,
            optional_params: &optional,
            messages: &[],
        },
    )
    .unwrap();
    assert_eq!(headers.get("x-api-key").map(String::as_str), Some("caller"));
    assert_eq!(headers.get("authorization"), None);
}

#[rstest]
#[case::default_base(&[], "https://api.edenai.run/v3/v1/messages")]
#[case::environment_base(
    &[(API_BASE_ENV, "https://gateway.example/v3")],
    "https://gateway.example/v3/v1/messages"
)]
fn complete_url_reads_the_environment_then_the_descriptor_base(
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let lookup = lookup(environment);
    let policy = policy();
    let credentials = MessagesCredentials {
        api_base: None,
        api_key: None,
        global_api_key: None,
        lookup: &lookup,
    };
    assert_eq!(policy.complete_url(&credentials).unwrap(), expected);
}

#[test]
fn complete_url_prefers_an_explicit_base_and_keeps_a_complete_messages_url() {
    let lookup = lookup(&[]);
    let policy = policy();
    let explicit = MessagesCredentials {
        api_base: Some("https://proxy.example/v3"),
        api_key: None,
        global_api_key: None,
        lookup: &lookup,
    };
    assert_eq!(
        policy.complete_url(&explicit).unwrap(),
        "https://proxy.example/v3/v1/messages"
    );
    let complete = MessagesCredentials {
        api_base: Some("https://proxy.example/v3/v1/messages"),
        api_key: None,
        global_api_key: None,
        lookup: &lookup,
    };
    assert_eq!(
        policy.complete_url(&complete).unwrap(),
        "https://proxy.example/v3/v1/messages"
    );
}

#[test]
fn transform_response_writes_the_cost_to_the_call_log() {
    let mut state = state();
    let record = transform_response(ReplyInput {
        body: br#"{"id":"msg_1","cost":0.5}"#,
        status: 200,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap();
    let reply = serde_json::to_value(&record).unwrap();
    assert_eq!(reply["id"], Value::from("msg_1"));
    assert_eq!(reply["cost"], Value::from(0.5_f64));
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    assert_eq!(details["response_cost"], Value::from(0.5_f64));
}

#[test]
fn transform_response_reads_the_cost_through_the_header_alias() {
    let mut state = state();
    let record = transform_response(ReplyInput {
        body: br#"{"id":"msg_1","x-edenai-cost":"2"}"#,
        status: 200,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap();
    assert_eq!(record.fields.get("x-edenai-cost").unwrap().get(), "\"2\"");
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    assert_eq!(details["response_cost"], Value::from(2_i32));
}

#[test]
fn transform_response_without_a_cost_leaves_the_call_log_alone() {
    let mut state = state();
    state.model_call_details.insert(
        String::from("response_cost"),
        HiddenJson::Number(Number::from(1_u64)),
    );
    transform_response(ReplyInput {
        body: br#"{"id":"msg_1"}"#,
        status: 200,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap();
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    assert_eq!(details["response_cost"], Value::from(1_i32));
}

#[test]
fn transform_response_lets_a_null_cost_beat_the_header_alias() {
    let mut state = state();
    transform_response(ReplyInput {
        body: br#"{"id":"msg_1","cost":null,"x-edenai-cost":2}"#,
        status: 200,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap();
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    assert_eq!(details["response_cost"], Value::Null);
}

#[test]
fn transform_response_ignores_an_invalid_cost() {
    let mut state = state();
    transform_response(ReplyInput {
        body: br#"{"id":"msg_1","cost":"abc"}"#,
        status: 200,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap();
    let details = serde_json::to_value(&state.model_call_details).unwrap();
    assert_eq!(details["response_cost"], Value::Null);
}

#[test]
fn transform_response_rejects_bytes_that_are_not_json() {
    let mut state = state();
    let error = transform_response(ReplyInput {
        body: b"not json",
        status: 502,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap_err();
    assert!(matches!(&error, EdenAiMessagesError::Response(_)));
    assert_eq!(error.status_code(), Some(502));
    assert_eq!(error.class_name(), "AnthropicError");
}

#[test]
fn transform_response_rejects_a_json_body_that_is_not_an_object() {
    let mut state = state();
    let error = transform_response(ReplyInput {
        body: b"[]",
        status: 200,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap_err();
    assert!(matches!(&error, EdenAiMessagesError::Response(_)));
    assert_eq!(error.status_code(), Some(500));
    assert!(error.to_string().contains("not a JSON object"), "{error}");
}

#[test]
fn transform_response_keeps_fields_it_does_not_model() {
    let mut state = state();
    let record = transform_response(ReplyInput {
        body: br#"{"id":"msg_1","custom_field":{"nested":true}}"#,
        status: 200,
        limits: &ApiLimits::default(),
        state: &mut state,
    })
    .unwrap();
    let reply = serde_json::to_value(&record).unwrap();
    assert_eq!(reply["custom_field"]["nested"], Value::from(true));
}
