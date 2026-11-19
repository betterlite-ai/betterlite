#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::ExposeSecret as _;

use super::{ProviderInfo, provider_info, validate_environment};
use crate::ragflow_chat::error::RagflowError;
use crate::ragflow_chat::tests::support::{Credentials, environment, key, object};

fn provider(case: &Credentials) -> Result<ProviderInfo, RagflowError> {
    provider_info(&case.input())
}

fn base_of(case: &Credentials) -> Option<String> {
    provider(case)
        .expect("the fixture provider info builds")
        .api_base
}

fn key_of(case: &Credentials) -> Option<String> {
    provider(case)
        .expect("the fixture provider info builds")
        .api_key
        .map(|value| value.expose_secret().to_owned())
}

fn validated(case: &Credentials) -> (BTreeMap<String, String>, Object) {
    let mut headers = BTreeMap::new();
    let mut litellm_params = Object::new();
    validate_environment(&case.input(), &mut headers, &mut litellm_params)
        .expect("the fixture environment validates");
    (headers, litellm_params)
}

#[test]
fn provider_info_parses_the_model_before_reading_the_base() {
    let mut case = Credentials::new("ragflow/chat");
    case.environment = environment(&[("RAGFLOW_API_BASE", "http://env:9380")]);
    let error = provider(&case).expect_err("a short model is refused");
    assert!(matches!(error, RagflowError::ModelTooShort { .. }));
}

#[test]
fn provider_info_reports_an_unknown_endpoint_type() {
    let case = Credentials::new("ragflow/completion/abc/m");
    let error = provider(&case).expect_err("an unknown endpoint type is refused");
    assert!(matches!(error, RagflowError::EndpointType { .. }));
}

#[test]
fn provider_info_returns_the_explicit_base_and_key_first() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.api_base = Some(String::from("http://explicit:9380"));
    case.api_key = Some(key("explicit-key"));
    case.global_api_base = Some(String::from("http://global:9380"));
    case.global_api_key = Some(key("global-key"));
    case.environment = environment(&[
        ("RAGFLOW_API_BASE", "http://env:9380"),
        ("RAGFLOW_API_KEY", "env-key"),
    ]);
    assert_eq!(base_of(&case), Some(String::from("http://explicit:9380")));
    assert_eq!(key_of(&case), Some(String::from("explicit-key")));
}

#[test]
fn an_empty_explicit_base_and_key_fall_through_to_the_globals() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.api_base = Some(String::new());
    case.api_key = Some(key(""));
    case.global_api_base = Some(String::from("http://global:9380"));
    case.global_api_key = Some(key("global-key"));
    case.environment = environment(&[
        ("RAGFLOW_API_BASE", "http://env:9380"),
        ("RAGFLOW_API_KEY", "env-key"),
    ]);
    assert_eq!(base_of(&case), Some(String::from("http://global:9380")));
    assert_eq!(key_of(&case), Some(String::from("global-key")));
}

#[test]
fn an_empty_global_falls_through_to_the_environment() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.api_base = Some(String::new());
    case.api_key = Some(key(""));
    case.global_api_base = Some(String::new());
    case.global_api_key = Some(key(""));
    case.environment = environment(&[
        ("RAGFLOW_API_BASE", "http://env:9380"),
        ("RAGFLOW_API_KEY", "env-key"),
    ]);
    assert_eq!(base_of(&case), Some(String::from("http://env:9380")));
    assert_eq!(key_of(&case), Some(String::from("env-key")));
}

#[test]
fn no_base_or_key_source_resolves_to_none() {
    let case = Credentials::new("ragflow/chat/abc/m");
    assert_eq!(base_of(&case), None);
    assert_eq!(key_of(&case), None);
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.environment = environment(&[("RAGFLOW_API_BASE", ""), ("RAGFLOW_API_KEY", "")]);
    assert_eq!(base_of(&case), None);
    assert_eq!(key_of(&case), None);
}

#[test]
fn the_resolved_key_is_not_shown_by_debug() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.api_key = Some(key("sk-secret-value"));
    let info = provider(&case).expect("the fixture provider info builds");
    let debug = format!("{info:?}");
    assert!(!debug.contains("sk-secret-value"));
}

#[test]
fn validate_environment_adds_the_bearer_header_from_the_chain() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.api_key = Some(key("explicit-key"));
    case.global_api_key = Some(key("global-key"));
    case.environment = environment(&[("RAGFLOW_API_KEY", "env-key")]);
    let (explicit_headers, _) = validated(&case);
    assert_eq!(
        explicit_headers.get("Authorization"),
        Some(&String::from("Bearer explicit-key"))
    );

    case.api_key = Some(key(""));
    let (global_headers, _) = validated(&case);
    assert_eq!(
        global_headers.get("Authorization"),
        Some(&String::from("Bearer global-key"))
    );

    case.global_api_key = None;
    let (environment_headers, _) = validated(&case);
    assert_eq!(
        environment_headers.get("Authorization"),
        Some(&String::from("Bearer env-key"))
    );
}

#[test]
fn validate_environment_without_a_key_adds_no_authorization() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.environment = environment(&[("RAGFLOW_API_KEY", "")]);
    let (headers, _params) = validated(&case);
    assert_eq!(headers.get("Authorization"), None);
    assert_eq!(
        headers.get("Content-Type"),
        Some(&String::from("application/json"))
    );
}

#[test]
fn validate_environment_reads_no_key_from_litellm_params() {
    let case = Credentials::new("ragflow/chat/abc/m");
    let mut headers = BTreeMap::new();
    let mut litellm_params = object(r#"{"api_key": "params-key"}"#);
    validate_environment(&case.input(), &mut headers, &mut litellm_params)
        .expect("the fixture environment validates");
    assert_eq!(headers.get("Authorization"), None);
}

#[test]
fn validate_environment_keeps_an_existing_content_type() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.api_key = Some(key("explicit-key"));
    let mut headers = BTreeMap::from([(String::from("Content-Type"), String::from("text/plain"))]);
    let mut litellm_params = Object::new();
    validate_environment(&case.input(), &mut headers, &mut litellm_params)
        .expect("the fixture environment validates");
    assert_eq!(
        headers.get("Content-Type"),
        Some(&String::from("text/plain"))
    );
    assert_eq!(headers.get("content-type"), None);
}

#[test]
fn validate_environment_keeps_a_lowercase_content_type() {
    let case = Credentials::new("ragflow/chat/abc/m");
    let mut headers = BTreeMap::from([(String::from("content-type"), String::from("text/plain"))]);
    let mut litellm_params = Object::new();
    validate_environment(&case.input(), &mut headers, &mut litellm_params)
        .expect("the fixture environment validates");
    assert_eq!(
        headers.get("content-type"),
        Some(&String::from("text/plain"))
    );
    assert_eq!(headers.get("Content-Type"), None);
}

#[test]
fn a_differently_spelled_content_type_does_not_count_as_present() {
    let case = Credentials::new("ragflow/chat/abc/m");
    let mut headers = BTreeMap::from([(String::from("Content-type"), String::from("text/plain"))]);
    let mut litellm_params = Object::new();
    validate_environment(&case.input(), &mut headers, &mut litellm_params)
        .expect("the fixture environment validates");
    assert_eq!(
        headers.get("Content-Type"),
        Some(&String::from("application/json"))
    );
    assert_eq!(
        headers.get("Content-type"),
        Some(&String::from("text/plain"))
    );
}

#[test]
fn validate_environment_stores_the_parsed_model() {
    let mut case = Credentials::new("ragflow/chat/abc/openai/gpt-4o");
    let (headers, params) = validated(&case);
    assert!(headers.contains_key("Content-Type"));
    assert_eq!(
        params.get("_ragflow_actual_model"),
        Some(&Json::Text(String::from("openai/gpt-4o")))
    );

    case.model = String::from("ragflow/agent/abc/glm-4");
    let (_, agent_params) = validated(&case);
    assert_eq!(
        agent_params.get("_ragflow_actual_model"),
        Some(&Json::Text(String::from("glm-4")))
    );
}

#[test]
fn validate_environment_swallows_a_bad_model() {
    let case = Credentials::new("ragflow/completion/abc/m");
    let (headers, params) = validated(&case);
    assert_eq!(
        headers.get("Content-Type"),
        Some(&String::from("application/json"))
    );
    assert_eq!(params.get("_ragflow_actual_model"), None);
}

#[test]
fn validate_environment_keeps_an_earlier_stored_model_on_a_bad_name() {
    let case = Credentials::new("ragflow/completion/abc/m");
    let mut headers = BTreeMap::new();
    let mut litellm_params = object(r#"{"_ragflow_actual_model": "earlier"}"#);
    validate_environment(&case.input(), &mut headers, &mut litellm_params)
        .expect("the fixture environment validates");
    assert_eq!(
        litellm_params.get("_ragflow_actual_model"),
        Some(&Json::Text(String::from("earlier")))
    );
}
