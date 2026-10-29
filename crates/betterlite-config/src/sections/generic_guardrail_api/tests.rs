#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;

const GENERIC_CONFIG: &str = "guardrails:\n  - guardrail_name: safety\n    litellm_params:\n      guardrail: generic_guardrail_api\n      api_base: http://localhost:8080\n";

fn resolve(
    source: &str,
    lookup: &BTreeMap<String, String>,
) -> Result<super::Settings, ConfigError> {
    let (raw, _ignored) = parse_view::<super::Raw>(source)?;
    raw.convert(lookup)
}

#[test]
fn generic_guardrail_api_config_names_the_guardrail_for_the_ui() {
    assert_eq!(super::UI_FRIENDLY_NAME, "Generic Guardrail API");
}

#[test]
fn generic_guardrail_api_config_skips_other_guardrail_kinds() {
    let source = GENERIC_CONFIG.replace("guardrail: generic_guardrail_api", "guardrail: aim");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert!(settings.guardrails.is_empty());
}

#[test]
fn generic_guardrail_api_config_is_optional() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    loader
        .load("general_settings:\n  master_key: test-master\n")
        .unwrap();
}

#[test]
fn generic_guardrail_api_config_requires_api_base() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let source = GENERIC_CONFIG.replace("      api_base: http://localhost:8080\n", "");
    let error = loader.load(&source).unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, path }
        if variable == "GENERIC_GUARDRAIL_API_BASE" && path == "guardrails.0.litellm_params.api_base")
    );
}

#[test]
fn generic_guardrail_api_config_api_key_is_optional() {
    let settings = resolve(GENERIC_CONFIG, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.guardrail_name, "safety");
    assert!(configured.api_key.is_none());
    assert_eq!(
        configured.api_base,
        "http://localhost:8080/beta/litellm_basic_guardrail_api"
    );
    assert_eq!(
        configured.unreachable_fallback,
        super::UnreachableFallback::FailClosed
    );
    assert!(configured.fail_on_error);
    assert_eq!(configured.provider_params, None);
}

#[test]
fn generic_guardrail_api_config_reads_additional_provider_specific_params() {
    let source = format!(
        "{GENERIC_CONFIG}      additional_provider_specific_params:\n        custom_threshold: 0.8\n        enable_feature: true\n"
    );
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    let params = configured.provider_params.as_ref().unwrap();
    assert_eq!(
        params.get("custom_threshold"),
        Some(&super::GuardrailValue::Decimal(0.8))
    );
    assert_eq!(
        params.get("enable_feature"),
        Some(&super::GuardrailValue::Boolean(true))
    );
}

#[test]
fn generic_guardrail_api_config_appends_path_only_once() {
    let source = GENERIC_CONFIG.replace(
        "http://localhost:8080",
        "http://localhost:8080/beta/litellm_basic_guardrail_api",
    );
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(
        configured.api_base,
        "http://localhost:8080/beta/litellm_basic_guardrail_api"
    );
}

#[test]
fn generic_guardrail_api_config_loads_environment() {
    let lookup = BTreeMap::from([
        ("GENERIC_GUARDRAIL_API_KEY".to_owned(), "key".to_owned()),
        (
            "GENERIC_GUARDRAIL_API_BASE".to_owned(),
            "http://env.example".to_owned(),
        ),
    ]);
    let source = GENERIC_CONFIG.replace("      api_base: http://localhost:8080\n", "");
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_key.as_ref().unwrap().expose_secret(), "key");
    assert_eq!(
        configured.api_base,
        "http://env.example/beta/litellm_basic_guardrail_api"
    );
}

#[test]
fn generic_guardrail_api_config_explicit_values_win() {
    let lookup = BTreeMap::from([(
        "GENERIC_GUARDRAIL_API_BASE".to_owned(),
        "http://env.example".to_owned(),
    )]);
    let source = format!(
        "{GENERIC_CONFIG}      fail_on_error: false\n      unreachable_fallback: fail_open\n"
    );
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(
        configured.api_base,
        "http://localhost:8080/beta/litellm_basic_guardrail_api"
    );
    assert!(!configured.fail_on_error);
    assert_eq!(
        configured.unreachable_fallback,
        super::UnreachableFallback::FailOpen
    );
}

#[test]
fn generic_guardrail_api_config_retains_unknown_paths() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let source = format!("{GENERIC_CONFIG}      misspelled: true\n");
    let error = loader.load(&source).unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { paths }
        if paths == ["guardrails.0.litellm_params.misspelled"]));
}
