#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, parse_view};
use crate::sections::Section as _;

use super::{Settings, UnreachableFallback};

fn resolve(source: &str, lookup: &BTreeMap<String, String>) -> Result<Settings, ConfigError> {
    let (raw, _ignored) = parse_view::<super::Raw>(source)?;
    raw.convert(lookup)
}

fn configured(settings: &Settings) -> &super::Configured {
    settings.guardrails.first().expect("one headroom guardrail")
}

#[test]
fn headroom_config_skips_other_guardrails_and_absent_sections() {
    let other = "guardrails:\n  - guardrail_name: aim\n    litellm_params:\n      guardrail: aim\n";
    assert!(
        resolve(other, &BTreeMap::new())
            .unwrap()
            .guardrails
            .is_empty()
    );
    let absent = resolve("general_settings:\n  master_key: key\n", &BTreeMap::new()).unwrap();
    assert!(absent.guardrails.is_empty());
}

#[test]
fn headroom_config_reads_every_key() {
    let source = "guardrails:\n  - guardrail_name: headroom-compress\n    litellm_params:\n      guardrail: headroom\n      api_base: https://headroom.test\n      api_key: secret-key\n      model: compressor\n      unreachable_fallback: fail_open\n      timeout: 2.5\n      ccr_retrieval: false\n";
    let settings = resolve(source, &BTreeMap::new()).unwrap();
    let headroom = configured(&settings);
    assert_eq!(headroom.guardrail_name, "headroom-compress");
    assert_eq!(headroom.api_base.as_deref(), Some("https://headroom.test"));
    assert_eq!(
        headroom
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("secret-key".to_owned())
    );
    assert_eq!(headroom.model.as_deref(), Some("compressor"));
    assert_eq!(
        headroom.unreachable_fallback,
        Some(UnreachableFallback::FailOpen)
    );
    assert_eq!(headroom.timeout, Some(2.5_f64));
    assert_eq!(headroom.ccr_retrieval, Some(false));
}

#[test]
fn headroom_config_resolves_environment_references() {
    let mut lookup = BTreeMap::new();
    lookup.insert("HEADROOM_BASE".to_owned(), "https://env.test".to_owned());
    lookup.insert("HEADROOM_KEY".to_owned(), "env-key".to_owned());
    lookup.insert("HEADROOM_MODEL".to_owned(), "env-model".to_owned());
    lookup.insert("HEADROOM_TIMEOUT".to_owned(), "7.5".to_owned());
    lookup.insert("HEADROOM_FALLBACK".to_owned(), "fail_closed".to_owned());
    lookup.insert("HEADROOM_CCR".to_owned(), "true".to_owned());
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      api_base: os.environ/HEADROOM_BASE\n      api_key: os.environ/HEADROOM_KEY\n      model: os.environ/HEADROOM_MODEL\n      unreachable_fallback: os.environ/HEADROOM_FALLBACK\n      timeout: os.environ/HEADROOM_TIMEOUT\n      ccr_retrieval: os.environ/HEADROOM_CCR\n";
    let settings = resolve(source, &lookup).unwrap();
    let headroom = configured(&settings);
    assert_eq!(headroom.api_base.as_deref(), Some("https://env.test"));
    assert_eq!(
        headroom
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("env-key".to_owned())
    );
    assert_eq!(headroom.model.as_deref(), Some("env-model"));
    assert_eq!(
        headroom.unreachable_fallback,
        Some(UnreachableFallback::FailClosed)
    );
    assert_eq!(headroom.timeout, Some(7.5_f64));
    assert_eq!(headroom.ccr_retrieval, Some(true));
}

#[test]
fn headroom_config_leaves_unset_keys_unset() {
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n";
    let settings = resolve(source, &BTreeMap::new()).unwrap();
    let headroom = configured(&settings);
    assert!(headroom.api_base.is_none());
    assert!(headroom.api_key.is_none());
    assert!(headroom.model.is_none());
    assert!(headroom.unreachable_fallback.is_none());
    assert!(headroom.timeout.is_none());
    assert!(headroom.ccr_retrieval.is_none());
}

#[test]
fn headroom_config_treats_an_empty_environment_reference_as_unset() {
    let mut lookup = BTreeMap::new();
    lookup.insert("HEADROOM_BASE".to_owned(), String::new());
    lookup.insert("HEADROOM_MODEL".to_owned(), String::new());
    lookup.insert("HEADROOM_FALLBACK".to_owned(), String::new());
    lookup.insert("HEADROOM_CCR".to_owned(), String::new());
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      api_base: os.environ/HEADROOM_BASE\n      model: os.environ/HEADROOM_MODEL\n      unreachable_fallback: os.environ/HEADROOM_FALLBACK\n      ccr_retrieval: os.environ/HEADROOM_CCR\n";
    let settings = resolve(source, &lookup).unwrap();
    let headroom = configured(&settings);
    assert!(headroom.api_base.is_none());
    assert!(headroom.model.is_none());
    assert!(headroom.unreachable_fallback.is_none());
    assert!(headroom.ccr_retrieval.is_none());
}

#[test]
fn headroom_config_reads_a_false_flag_from_an_environment_reference() {
    let mut lookup = BTreeMap::new();
    lookup.insert("HEADROOM_CCR".to_owned(), "off".to_owned());
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      ccr_retrieval: os.environ/HEADROOM_CCR\n";
    let settings = resolve(source, &lookup).unwrap();
    assert_eq!(configured(&settings).ccr_retrieval, Some(false));
}

#[test]
fn headroom_config_rejects_a_flag_that_is_not_a_bool() {
    let mut lookup = BTreeMap::new();
    lookup.insert("HEADROOM_CCR".to_owned(), "maybe".to_owned());
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      ccr_retrieval: os.environ/HEADROOM_CCR\n";
    let error = resolve(source, &lookup).unwrap_err();
    assert!(error.to_string().contains("HEADROOM_CCR"), "{error}");
}

#[test]
fn headroom_config_keeps_an_explicit_empty_value() {
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      model: \"\"\n";
    let settings = resolve(source, &BTreeMap::new()).unwrap();
    assert_eq!(configured(&settings).model.as_deref(), Some(""));
}

#[test]
fn headroom_config_rejects_an_unknown_fallback() {
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      unreachable_fallback: fail_loud\n";
    let error = resolve(source, &BTreeMap::new()).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("guardrails.0.litellm_params.unreachable_fallback"),
        "{message}"
    );
    assert!(message.contains("fail_loud"), "{message}");
}

#[test]
fn headroom_config_reports_a_missing_environment_variable() {
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      api_base: os.environ/MISSING_HEADROOM_BASE\n";
    let error = resolve(source, &BTreeMap::new()).unwrap_err();
    assert!(
        matches!(
            &error,
            ConfigError::MissingEnvironment { variable, .. } if variable == "MISSING_HEADROOM_BASE"
        ),
        "{error}"
    );
}

#[test]
fn headroom_config_rejects_a_non_numeric_timeout() {
    let source = "guardrails:\n  - guardrail_name: headroom\n    litellm_params:\n      guardrail: headroom\n      timeout: soon\n";
    let error = resolve(source, &BTreeMap::new()).unwrap_err();
    assert!(error.to_string().contains("timeout"), "{error}");
}
