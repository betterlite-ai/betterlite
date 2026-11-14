#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    BYPASS_HEADER, CONNECT_TIMEOUT_CAP, DEFAULT_COMPRESS_TIMEOUT, HEADROOM_GUARDRAIL_PROVIDER,
    HEADROOM_RETRIEVE_TOOL_NAME, HeadroomConfig, HeadroomInput, HeadroomSettings,
    HeadroomSettingsError, SUPPORTED_EVENT_HOOKS, UnreachableFallback, resolve_timeout,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(input: &HeadroomInput, env: &BTreeMap<String, String>) -> HeadroomSettings {
    HeadroomSettings::new(input, env).expect("settings")
}

#[test]
fn settings_constants_match_litellm() {
    assert_eq!(BYPASS_HEADER, "x-headroom-bypass");
    assert_eq!(HEADROOM_RETRIEVE_TOOL_NAME, "headroom_retrieve");
    assert_eq!(HEADROOM_GUARDRAIL_PROVIDER, "headroom");
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall]
    );
    assert_eq!(DEFAULT_COMPRESS_TIMEOUT, Duration::from_secs(60));
    assert_eq!(CONNECT_TIMEOUT_CAP, Duration::from_secs(5));
}

#[test]
fn settings_use_the_explicit_values_and_default_the_rest() {
    let input = HeadroomInput {
        api_base: Some("https://headroom.test".to_owned()),
        api_key: Some(SecretString::from("explicit-key")),
        model: Some("compress-model".to_owned()),
        unreachable_fallback: Some("fail_open".to_owned()),
        timeout: Some(2.0),
        ccr_retrieval: Some(false),
    };
    let resolved = settings(&input, &BTreeMap::new());
    assert_eq!(resolved.api_base, "https://headroom.test");
    assert_eq!(
        resolved
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("explicit-key".to_owned())
    );
    assert_eq!(resolved.model.as_deref(), Some("compress-model"));
    assert_eq!(resolved.unreachable_fallback, UnreachableFallback::FailOpen);
    assert_eq!(resolved.timeout.total, Duration::from_secs(2));
    assert_eq!(resolved.timeout.connect, Duration::from_secs(2));
    assert!(!resolved.ccr_retrieval);
}

#[test]
fn settings_default_to_fail_closed_and_ccr_retrieval_on() {
    let input = HeadroomInput {
        api_base: Some("https://headroom.test".to_owned()),
        api_key: None,
        model: None,
        unreachable_fallback: None,
        timeout: None,
        ccr_retrieval: None,
    };
    let resolved = settings(&input, &BTreeMap::new());
    assert_eq!(
        resolved.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
    assert!(resolved.ccr_retrieval);
    assert!(resolved.api_key.is_none());
    assert!(resolved.model.is_none());
    assert_eq!(resolved.timeout, resolve_timeout(None));
}

#[test]
fn settings_fall_back_to_the_environment_secrets() {
    let env = environment(&[
        ("HEADROOM_API_BASE", "https://env.test///"),
        ("HEADROOM_API_KEY", "env-key"),
    ]);
    let resolved = settings(&HeadroomInput::default(), &env);
    assert_eq!(resolved.api_base, "https://env.test");
    assert_eq!(
        resolved
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("env-key".to_owned())
    );
}

#[test]
fn settings_treat_an_empty_value_as_unset() {
    let env = environment(&[
        ("HEADROOM_API_BASE", "https://env.test"),
        ("HEADROOM_API_KEY", "env-key"),
    ]);
    let input = HeadroomInput {
        api_base: Some(String::new()),
        api_key: Some(SecretString::from(String::new())),
        model: None,
        unreachable_fallback: None,
        timeout: None,
        ccr_retrieval: None,
    };
    let resolved = settings(&input, &env);
    assert_eq!(resolved.api_base, "https://env.test");
    assert_eq!(
        resolved
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("env-key".to_owned())
    );
}

#[test]
fn settings_treat_an_empty_environment_key_as_unset() {
    let env = environment(&[("HEADROOM_API_KEY", "")]);
    let input = HeadroomInput {
        api_base: Some("https://headroom.test".to_owned()),
        api_key: None,
        model: None,
        unreachable_fallback: None,
        timeout: None,
        ccr_retrieval: None,
    };
    assert!(settings(&input, &env).api_key.is_none());
}

#[test]
fn settings_strip_every_trailing_slash() {
    let input = HeadroomInput {
        api_base: Some("https://headroom.test/base/".to_owned()),
        api_key: None,
        model: None,
        unreachable_fallback: None,
        timeout: None,
        ccr_retrieval: None,
    };
    assert_eq!(
        settings(&input, &BTreeMap::new()).api_base,
        "https://headroom.test/base"
    );
}

#[rstest]
#[case::missing(None, BTreeMap::from([("HEADROOM_API_KEY".to_owned(), "key".to_owned())]))]
#[case::empty(Some(""), BTreeMap::new())]
#[case::slashes_only(Some("/"), BTreeMap::new())]
fn settings_require_an_api_base(
    #[case] api_base: Option<&str>,
    #[case] env: BTreeMap<String, String>,
) {
    let input = HeadroomInput {
        api_base: api_base.map(str::to_owned),
        api_key: None,
        model: None,
        unreachable_fallback: None,
        timeout: None,
        ccr_retrieval: None,
    };
    let error = HeadroomSettings::new(&input, &env).unwrap_err();
    assert!(matches!(error, HeadroomSettingsError::MissingApiBase));
    assert_eq!(
        error.to_string(),
        "Headroom guardrail requires an API base URL. Set `api_base` in the guardrail config or HEADROOM_API_BASE env var."
    );
}

#[rstest]
#[case::exact("fail_open", UnreachableFallback::FailOpen)]
#[case::closed("fail_closed", UnreachableFallback::FailClosed)]
#[case::upper_case("FAIL_OPEN", UnreachableFallback::FailClosed)]
#[case::empty("", UnreachableFallback::FailClosed)]
#[case::other("failover", UnreachableFallback::FailClosed)]
fn settings_accept_fail_open_only_as_the_exact_text(
    #[case] fallback: &str,
    #[case] expected: UnreachableFallback,
) {
    let input = HeadroomInput {
        api_base: Some("https://headroom.test".to_owned()),
        api_key: None,
        model: None,
        unreachable_fallback: Some(fallback.to_owned()),
        timeout: None,
        ccr_retrieval: None,
    };
    assert_eq!(
        settings(&input, &BTreeMap::new()).unreachable_fallback,
        expected
    );
}

#[test]
fn timeout_defaults_to_sixty_seconds_with_a_five_second_connect_cap() {
    assert_eq!(
        resolve_timeout(None),
        super::CompressTimeout {
            total: Duration::from_secs(60),
            connect: Duration::from_secs(5),
        }
    );
}

#[rstest]
#[case::zero(0.0)]
#[case::negative(-3.0)]
#[case::nan(f64::NAN)]
#[case::infinite(f64::INFINITY)]
#[case::negative_infinite(f64::NEG_INFINITY)]
fn timeout_rejects_unusable_values(#[case] seconds: f64) {
    let resolved = resolve_timeout(Some(seconds));
    assert_eq!(resolved.total, Duration::from_secs(60));
    assert_eq!(resolved.connect, Duration::from_secs(5));
}

#[test]
fn timeout_keeps_a_value_below_the_connect_cap() {
    let resolved = resolve_timeout(Some(2.0));
    assert_eq!(resolved.total, Duration::from_secs(2));
    assert_eq!(resolved.connect, Duration::from_secs(2));
}

#[test]
fn timeout_caps_the_connect_budget_at_five_seconds() {
    let resolved = resolve_timeout(Some(90.0));
    assert_eq!(resolved.total, Duration::from_secs(90));
    assert_eq!(resolved.connect, Duration::from_secs(5));
}

#[test]
fn request_headers_carry_the_bearer_key_only_when_a_key_is_set() {
    let input = HeadroomInput {
        api_base: Some("https://headroom.test".to_owned()),
        api_key: None,
        model: None,
        unreachable_fallback: None,
        timeout: None,
        ccr_retrieval: None,
    };
    let headers = settings(&input, &BTreeMap::new())
        .request_headers()
        .unwrap();
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
    assert!(headers.get(AUTHORIZATION).is_none());

    let with_key = HeadroomInput {
        api_base: Some("https://headroom.test".to_owned()),
        api_key: Some(SecretString::from("secret-key")),
        model: None,
        unreachable_fallback: None,
        timeout: None,
        ccr_retrieval: None,
    };
    let headers = settings(&with_key, &BTreeMap::new())
        .request_headers()
        .unwrap();
    assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer secret-key");
    assert!(headers.get(AUTHORIZATION).unwrap().is_sensitive());
}

#[test]
fn config_defaults_match_the_litellm_model() {
    let config: HeadroomConfig = serde_json::from_str("{}").unwrap();
    assert!(config.api_base.is_none());
    assert!(config.api_key.is_none());
    assert!(config.model.is_none());
    assert_eq!(config.unreachable_fallback, UnreachableFallback::FailClosed);
    assert!(config.ccr_retrieval);
    assert_eq!(HeadroomConfig::ui_friendly_name(), "Headroom");
}

#[test]
fn config_keeps_the_explicit_values() {
    let config: HeadroomConfig = serde_json::from_str(
        r#"{"api_base": "https://headroom.test", "api_key": "k", "model": "m", "unreachable_fallback": "fail_open", "ccr_retrieval": false}"#,
    )
    .unwrap();
    assert_eq!(config.api_base.as_deref(), Some("https://headroom.test"));
    assert_eq!(
        config
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("k".to_owned())
    );
    assert_eq!(config.model.as_deref(), Some("m"));
    assert_eq!(config.unreachable_fallback, UnreachableFallback::FailOpen);
    assert!(!config.ccr_retrieval);
}

#[rstest]
#[case::unknown("failover")]
#[case::wrong_case("fail_open ")]
fn config_rejects_an_unsupported_unreachable_fallback(#[case] fallback: &str) {
    let source = format!(r#"{{"unreachable_fallback": "{fallback}"}}"#);
    let error = serde_json::from_str::<HeadroomConfig>(&source)
        .err()
        .expect("an unsupported fallback is a config error");
    assert!(
        error.to_string().contains("unreachable_fallback"),
        "error names the field: {error}"
    );
}
