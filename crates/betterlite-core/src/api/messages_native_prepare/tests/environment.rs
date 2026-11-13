use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{headers, json, object, secret, settings};
use crate::api::anthropic_beta_filter::native_injection::NativeRequest;
use crate::api::messages_native_prepare::environment::validate_environment;
use crate::api::messages_native_prepare::error::MessagesPrepareError;

fn validated(
    given: &[(&str, &str)],
    api_key: Option<&str>,
    api_base: Option<&str>,
) -> (IndexMap<String, String>, Option<String>) {
    let mut request_headers = headers(given);
    let optional_params = Object::new();
    let answer = validate_environment(
        &mut request_headers,
        api_key.map(secret),
        api_base,
        &NativeRequest {
            provider: "anthropic",
            optional_params: &optional_params,
            messages: &[],
        },
        &settings("https://api.anthropic.com"),
    )
    .expect("a credential is present");
    (request_headers, answer)
}

#[rstest]
#[case::caller_api_key(&[("X-API-Key", "caller")], "X-API-Key", "caller")]
#[case::caller_authorization(&[("Authorization", "Bearer caller")], "Authorization", "Bearer caller")]
fn environment_header_precedence_keeps_the_caller_credential(
    #[case] given: &[(&str, &str)],
    #[case] name: &str,
    #[case] value: &str,
) {
    let (result, returned_base) =
        validated(given, Some("sk-test"), Some("https://configured.test"));
    assert_eq!(returned_base, Some("https://configured.test".to_owned()));
    assert_eq!(result.get(name), Some(&value.to_owned()));
    assert_eq!(result.get("x-api-key"), None);
    assert_eq!(result.get("authorization"), None);
    assert_eq!(
        result.get("anthropic-version"),
        Some(&"2023-06-01".to_owned())
    );
    assert_eq!(
        result.get("content-type"),
        Some(&"application/json".to_owned())
    );
}

#[test]
fn environment_header_precedence_adds_the_only_credential_it_has() {
    let (result, returned_base) = validated(&[], Some("sk-test"), Some("https://configured.test"));
    assert_eq!(returned_base, Some("https://configured.test".to_owned()));
    assert_eq!(result.get("x-api-key"), Some(&"sk-test".to_owned()));
    assert_eq!(result.get("authorization"), None);
    assert_eq!(
        result.get("anthropic-version"),
        Some(&"2023-06-01".to_owned())
    );
    assert_eq!(
        result.get("content-type"),
        Some(&"application/json".to_owned())
    );
}

#[test]
fn environment_header_precedence_requires_a_credential() {
    let mut given = headers(&[("x-caller", "value")]);
    let optional_params = Object::new();
    let failure = validate_environment(
        &mut given,
        None,
        None,
        &NativeRequest {
            provider: "anthropic",
            optional_params: &optional_params,
            messages: &[],
        },
        &settings("https://api.anthropic.com"),
    )
    .expect_err("no api key and no auth token");
    assert_eq!(failure, MessagesPrepareError::MissingCredential);
    assert_eq!(failure.status_code(), Some(401));
    assert_eq!(failure.class_name(), "AuthenticationError");
    assert_eq!(
        failure.to_string(),
        "no Anthropic credential is set; set ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN"
    );
}

#[rstest]
#[case::speed(r#"{"speed":"fast"}"#, "fast-mode-2026-02-01")]
#[case::context_management(
    r#"{"context_management":{"edits":[{"type":"clear_tool_uses_20250919"}]}}"#,
    "context-management-2025-06-27"
)]
fn environment_header_precedence_injects_feature_betas(#[case] params: &str, #[case] beta: &str) {
    let mut given = headers(&[]);
    let optional_params = object(params);
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let answer = validate_environment(
        &mut given,
        Some(secret("sk-test")),
        None,
        &NativeRequest {
            provider: "anthropic",
            optional_params: &optional_params,
            messages: &messages,
        },
        &settings("https://api.anthropic.com"),
    )
    .expect("a credential is present");
    assert_eq!(answer, None);
    assert_eq!(given.get("x-api-key"), Some(&"sk-test".to_owned()));
    assert_eq!(given.get("anthropic-beta"), Some(&beta.to_owned()));
}

#[rstest]
#[case::lowercase_version(&[("anthropic-version", "2024-01-01")], "anthropic-version", "2024-01-01")]
#[case::lowercase_content_type(&[("content-type", "text/plain")], "content-type", "text/plain")]
fn environment_header_precedence_keeps_explicit_lowercase_values(
    #[case] given: &[(&str, &str)],
    #[case] name: &str,
    #[case] value: &str,
) {
    let (result, _returned_base) = validated(given, Some("sk-test"), None);
    assert_eq!(result.get(name), Some(&value.to_owned()));
}

#[test]
fn environment_header_precedence_coexists_with_mixed_case_versions() {
    let (result, _returned_base) = validated(
        &[("Anthropic-Version", "2024-01-01")],
        Some("sk-test"),
        None,
    );
    assert_eq!(
        result.get("Anthropic-Version"),
        Some(&"2024-01-01".to_owned())
    );
    assert_eq!(
        result.get("anthropic-version"),
        Some(&"2023-06-01".to_owned())
    );
}

#[rstest]
#[case::from_the_key(None)]
#[case::from_the_header(Some("Bearer sk-ant-oat-test"))]
fn environment_header_precedence_marks_oauth_tokens(#[case] authorization: Option<&str>) {
    let given: Vec<(&str, &str)> = authorization
        .map(|value| vec![("Authorization", value)])
        .unwrap_or_default();
    let api_key = match authorization {
        None => Some("sk-ant-oat-test"),
        Some(_) => None,
    };
    let (result, _returned_base) = validated(&given, api_key, None);
    assert_eq!(
        result.get("authorization"),
        Some(&"Bearer sk-ant-oat-test".to_owned())
    );
    assert_eq!(result.get("x-api-key"), None);
    assert_eq!(
        result.get("anthropic-beta"),
        Some(&"oauth-2025-04-20".to_owned())
    );
    assert_eq!(
        result.get("anthropic-dangerous-direct-browser-access"),
        Some(&"true".to_owned())
    );
    assert_eq!(
        result.get("anthropic-version"),
        Some(&"2023-06-01".to_owned())
    );
    assert_eq!(
        result.get("content-type"),
        Some(&"application/json".to_owned())
    );
}

#[test]
fn environment_header_precedence_keeps_existing_beta_tokens() {
    let (result, _returned_base) = validated(
        &[("anthropic-beta", "context-management-2025-06-27")],
        Some("sk-test"),
        None,
    );
    assert_eq!(
        result.get("anthropic-beta"),
        Some(&"context-management-2025-06-27".to_owned())
    );
}
