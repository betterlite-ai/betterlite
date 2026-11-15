#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::alice_conduct_decisions::Raw;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback as ConfigFallback;
use secrecy::ExposeSecret as _;

use crate::alice::{AliceConfigModel, DEFAULT_API_BASE};

#[test]
fn alice_guardrail_config_model_deserializes_the_litellm_fields() {
    let model: AliceConfigModel = serde_json::from_str(r#"{"api_base": "https://alice.example"}"#)
        .expect("the config model accepts an api_base");
    assert!(model.api_key.is_none());
    assert_eq!(model.api_base.as_deref(), Some("https://alice.example"));
    assert_eq!(AliceConfigModel::ui_friendly_name(), "Alice");
    let with_key: AliceConfigModel = serde_json::from_str(r#"{"api_key": "secret"}"#)
        .expect("the config model accepts an api_key");
    assert_eq!(
        with_key
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("secret".to_owned())
    );
}

#[test]
fn alice_guardrail_config_section_loads_values_and_environment() {
    let yaml = "
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
  - guardrail_name: not-alice
    litellm_params:
      guardrail: bedrock
";
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the section parses");
    let environment = BTreeMap::from([
        ("ALICE_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "ALICE_API_BASE".to_owned(),
            "https://env.example".to_owned(),
        ),
    ]);
    let settings = raw.convert(&environment).expect("the section converts");
    assert_eq!(settings.guardrails.len(), 1);
    let configured = &settings.guardrails[0];
    assert_eq!(configured.guardrail_name, "alice-check");
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("env-key".to_owned())
    );
    assert_eq!(configured.api_base.as_deref(), Some("https://env.example"));
    assert_eq!(configured.unreachable_fallback, ConfigFallback::FailClosed);
}

#[test]
fn alice_guardrail_config_section_prefers_config_and_defaults_are_litellm_s() {
    let yaml = "
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      api_key: config-key
      unreachable_fallback: fail_open
";
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the section parses");
    let settings = raw.convert(&BTreeMap::new()).expect("the section converts");
    let configured = &settings.guardrails[0];
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("config-key".to_owned())
    );
    assert_eq!(configured.api_base, None);
    assert_eq!(configured.unreachable_fallback, ConfigFallback::FailOpen);
    assert_eq!(DEFAULT_API_BASE, "https://api.alice.io");
}

#[test]
fn alice_guardrail_config_section_lowercases_the_fallback_value() {
    let yaml = "
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      api_key: config-key
      unreachable_fallback: FAIL_OPEN
";
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the section parses");
    let settings = raw.convert(&BTreeMap::new()).expect("the section converts");
    assert_eq!(
        settings.guardrails[0].unreachable_fallback,
        ConfigFallback::FailOpen
    );
}

#[test]
fn alice_guardrail_config_section_rejects_an_unknown_fallback_value() {
    let yaml = "
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      api_key: config-key
      unreachable_fallback: never
";
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the section parses");
    let error = raw
        .convert(&BTreeMap::new())
        .expect_err("an unknown fallback refuses the config");
    assert!(
        matches!(
            error,
            ConfigError::InvalidScalar { ref path, .. }
                if path == "guardrails.0.litellm_params.unreachable_fallback"
        ),
        "{error:?}"
    );
}
