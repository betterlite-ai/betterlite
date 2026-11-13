#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;

use super::{
    PROVIDER, ResponsesConfigError, SUPPORTS_NATIVE_WEBSOCKET, get_complete_url,
    validate_environment,
};
use crate::openai_base_config::Headers;

type Environment = BTreeMap<String, String>;

fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

#[test]
fn the_config_reports_openrouter_and_no_native_websocket() {
    assert_eq!(PROVIDER, ProviderId::Openrouter);
    assert_eq!(PROVIDER.to_string(), "openrouter");
    assert!(!SUPPORTS_NATIVE_WEBSOCKET);
}

#[test]
fn validate_environment_sets_only_the_bearer_header() {
    let given = Headers::from([(String::from("x-extra"), String::from("1"))]);
    let headers = validate_environment(given, Some("sk-or"), None, &environment(&[])).unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-or")
    );
    assert_eq!(headers.get("x-extra").map(String::as_str), Some("1"));
    assert!(!headers.contains_key("Content-Type"));
    assert!(!headers.contains_key("HTTP-Referer"));
    assert_eq!(headers.len(), 2);
}

#[rstest::rstest]
#[case::explicit_wins(
    Some("explicit"),
    Some("global"),
    &[("OPENROUTER_API_KEY", "env"), ("OR_API_KEY", "or")],
    "explicit"
)]
#[case::global_second(
    None,
    Some("global"),
    &[("OPENROUTER_API_KEY", "env"), ("OR_API_KEY", "or")],
    "global"
)]
#[case::openrouter_env_third(None, None, &[("OPENROUTER_API_KEY", "env"), ("OR_API_KEY", "or")], "env")]
#[case::or_api_key_last(None, None, &[("OR_API_KEY", "or")], "or")]
#[case::empty_explicit_is_unset(
    Some(""),
    None,
    &[("OPENROUTER_API_KEY", "env")],
    "env"
)]
fn validate_environment_follows_the_key_order(
    #[case] api_key: Option<&str>,
    #[case] global_api_key: Option<&str>,
    #[case] env: &[(&str, &str)],
    #[case] expected: &str,
) {
    let headers =
        validate_environment(Headers::new(), api_key, global_api_key, &environment(env)).unwrap();
    let wanted = format!("Bearer {expected}");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(wanted.as_str())
    );
}

#[test]
fn validate_environment_fails_with_the_litellm_text() {
    let error = validate_environment(Headers::new(), None, None, &environment(&[])).unwrap_err();
    assert_eq!(error, ResponsesConfigError::MissingApiKey);
    assert_eq!(
        error.to_string(),
        "OpenRouter API key is required. Set OPENROUTER_API_KEY environment variable or pass api_key parameter."
    );
}

#[rstest::rstest]
#[case::explicit(
    Some("https://proxy.example/v1"),
    Some("https://global.example"),
    &[],
    "https://proxy.example/v1/responses"
)]
#[case::global_second(
    None,
    Some("https://global.example/"),
    &[("OPENROUTER_API_BASE", "https://env.example")],
    "https://global.example/responses"
)]
#[case::env_third(
    None,
    None,
    &[("OPENROUTER_API_BASE", "https://env.example//")],
    "https://env.example/responses"
)]
#[case::default(None, None, &[], "https://openrouter.ai/api/v1/responses")]
#[case::empty_explicit_is_unset(None, None, &[("OPENROUTER_API_BASE", "")], "https://openrouter.ai/api/v1/responses")]
fn get_complete_url_follows_the_base_order(
    #[case] api_base: Option<&str>,
    #[case] global_api_base: Option<&str>,
    #[case] env: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(
        get_complete_url(api_base, global_api_base, &environment(env)),
        expected
    );
}
