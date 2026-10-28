#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;

const GRAYSWAN_CONFIG: &str = "guardrails:\n  - guardrail_name: safety\n    litellm_params:\n      guardrail: grayswan\n      api_key: configured-key\n";

fn resolve(
    source: &str,
    lookup: &BTreeMap<String, String>,
) -> Result<super::Settings, ConfigError> {
    let (raw, _ignored) = parse_view::<super::Raw>(source)?;
    raw.convert(lookup)
}

#[test]
fn grayswan_config_names_the_guardrail_for_the_ui() {
    assert_eq!(super::UI_FRIENDLY_NAME, "Gray Swan Cygnal");
}

#[test]
fn grayswan_config_skips_other_guardrail_kinds() {
    let source = GRAYSWAN_CONFIG.replace("guardrail: grayswan", "guardrail: aim");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert!(settings.guardrails.is_empty());
}

#[test]
fn grayswan_config_is_optional() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    loader
        .load("general_settings:\n  master_key: test-master\n")
        .unwrap();
}

#[test]
fn grayswan_config_requires_key_only_when_enabled() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let source = GRAYSWAN_CONFIG.replace("      api_key: configured-key\n", "");
    let error = loader.load(&source).unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, path }
        if variable == "GRAYSWAN_API_KEY" && path == "guardrails.0.litellm_params.api_key")
    );
}

#[test]
fn grayswan_config_defaults_match_litellm() {
    let settings = resolve(GRAYSWAN_CONFIG, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.guardrail_name, "safety");
    assert_eq!(configured.api_key.expose_secret(), "configured-key");
    assert_eq!(configured.api_base, "https://api.grayswan.ai");
    assert_eq!(
        configured.on_flagged_action,
        super::OnFlaggedAction::Monitor
    );
    assert!((configured.violation_threshold - 0.5).abs() < f64::EPSILON);
    assert_eq!(configured.reasoning_mode, None);
    assert_eq!(configured.categories, None);
    assert_eq!(configured.policy_id, None);
    assert!(configured.fail_open);
}

#[test]
fn grayswan_config_loads_environment() {
    let lookup = BTreeMap::from([
        ("GRAYSWAN_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "GRAYSWAN_API_BASE".to_owned(),
            "https://example.test/".to_owned(),
        ),
        ("GRAYSWAN_VIOLATION_THRESHOLD".to_owned(), "0.7".to_owned()),
        ("GRAYSWAN_REASONING_MODE".to_owned(), "hybrid".to_owned()),
    ]);
    let source = GRAYSWAN_CONFIG.replace("      api_key: configured-key\n", "");
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_key.expose_secret(), "env-key");
    assert_eq!(configured.api_base, "https://example.test");
    assert!((configured.violation_threshold - 0.7).abs() < f64::EPSILON);
    assert_eq!(configured.reasoning_mode.as_deref(), Some("hybrid"));
}

#[test]
fn grayswan_config_rejects_unsupported_reasoning_mode() {
    let source = format!("{GRAYSWAN_CONFIG}      reasoning_mode: verbose\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.reasoning_mode, None);
}

#[test]
fn grayswan_config_explicit_values_win() {
    let lookup = BTreeMap::from([(
        "GRAYSWAN_API_BASE".to_owned(),
        "https://env.example".to_owned(),
    )]);
    let source = format!(
        "{GRAYSWAN_CONFIG}      api_base: https://configured.example\n      on_flagged_action: BLOCK\n      violation_threshold: \"0.9\"\n      categories: [jailbreak]\n      policy_id: policy-1\n      fail_open: false\n"
    );
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_base, "https://configured.example");
    assert_eq!(configured.on_flagged_action, super::OnFlaggedAction::Block);
    assert!((configured.violation_threshold - 0.9).abs() < f64::EPSILON);
    assert_eq!(
        configured.categories.as_deref(),
        Some(["jailbreak".to_owned()].as_slice())
    );
    assert_eq!(configured.policy_id.as_deref(), Some("policy-1"));
    assert!(!configured.fail_open);
}

#[test]
fn grayswan_config_retains_unknown_paths() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let source = format!("{GRAYSWAN_CONFIG}      misspelled: true\n");
    let error = loader.load(&source).unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { paths }
        if paths == ["guardrails.0.litellm_params.misspelled"]));
}
