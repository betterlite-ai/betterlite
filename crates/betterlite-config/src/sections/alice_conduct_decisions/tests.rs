#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::{DEFAULT_API_BASE, Raw, Settings};
use crate::loader::ConfigError;
use crate::sections::Section;
use crate::sections::guardrail_params::base::UnreachableFallback;

fn settings_from(yaml: &str, env: &[(&str, &str)]) -> Settings {
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the alice section parses");
    let lookup: BTreeMap<String, String> = env
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    raw.convert(&lookup).expect("the alice section converts")
}

const ALICE_ONLY: &str = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
  - guardrail_name: other
    litellm_params:
      guardrail: bedrock
";

#[test]
fn alice_conduct_decisions_keeps_only_alice_entries() {
    let settings = settings_from(ALICE_ONLY, &[]);
    assert_eq!(settings.guardrails.len(), 1);
    assert_eq!(settings.guardrails[0].guardrail_name, "alice-check");
}

#[test]
fn alice_conduct_decisions_falls_back_to_the_alice_environment_names() {
    let settings = settings_from(
        ALICE_ONLY,
        &[
            ("ALICE_API_KEY", "env-key"),
            ("ALICE_API_BASE", "https://env.example"),
        ],
    );
    let configured = &settings.guardrails[0];
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("env-key")
    );
    assert_eq!(configured.api_base.as_deref(), Some("https://env.example"));
    assert_eq!(
        configured.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}

#[test]
fn alice_conduct_decisions_prefers_configured_values_over_the_environment() {
    let yaml = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      api_key: config-key
      api_base: https://config.example
      unreachable_fallback: fail_open
";
    let settings = settings_from(
        yaml,
        &[
            ("ALICE_API_KEY", "env-key"),
            ("ALICE_API_BASE", "https://env.example"),
        ],
    );
    let configured = &settings.guardrails[0];
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("config-key")
    );
    assert_eq!(
        configured.api_base.as_deref(),
        Some("https://config.example")
    );
    assert_eq!(
        configured.unreachable_fallback,
        UnreachableFallback::FailOpen
    );
}

#[test]
fn alice_conduct_decisions_resolves_environment_references_in_config_values() {
    let yaml = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      api_key: os.environ/SECRET_ALICE_KEY
";
    let settings = settings_from(yaml, &[("SECRET_ALICE_KEY", "resolved-key")]);
    let configured = &settings.guardrails[0];
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("resolved-key")
    );
}

#[test]
fn alice_conduct_decisions_leaves_a_missing_api_key_for_the_class_to_refuse() {
    let settings = settings_from(ALICE_ONLY, &[]);
    let configured = &settings.guardrails[0];
    assert!(configured.api_key.is_none());
    assert_eq!(configured.api_base, None);
    assert_eq!(DEFAULT_API_BASE, "https://api.alice.io");
}

#[test]
fn alice_conduct_decisions_treats_an_empty_environment_key_as_unset() {
    let settings = settings_from(ALICE_ONLY, &[("ALICE_API_KEY", "")]);
    assert!(settings.guardrails[0].api_key.is_none());
}

#[test]
fn alice_conduct_decisions_lowercases_the_fallback_value() {
    let yaml = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      unreachable_fallback: FAIL_OPEN
";
    let settings = settings_from(yaml, &[]);
    assert_eq!(
        settings.guardrails[0].unreachable_fallback,
        UnreachableFallback::FailOpen
    );
}

#[test]
fn alice_conduct_decisions_accepts_an_explicit_fail_closed_fallback() {
    let yaml = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      unreachable_fallback: fail_closed
";
    let settings = settings_from(yaml, &[]);
    assert_eq!(
        settings.guardrails[0].unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}

#[test]
fn alice_conduct_decisions_rejects_an_unknown_fallback_value() {
    let yaml = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      unreachable_fallback: never
";
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the alice section parses");
    let error = raw
        .convert(&BTreeMap::new())
        .expect_err("an unknown fallback refuses the config");
    assert!(
        matches!(
            error,
            ConfigError::InvalidScalar {
                ref path,
                value: false,
            } if path == "guardrails.0.litellm_params.unreachable_fallback"
        ),
        "{error:?}"
    );
}
