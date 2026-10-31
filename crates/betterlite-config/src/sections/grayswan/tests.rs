#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use secrecy::ExposeSecret as _;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use crate::loader::{ConfigError, ConfigLoader, error_category, parse_view};
use crate::sections::Section as _;

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

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
    assert!(configured.categories.is_empty());
    assert!(configured.policy_id.is_empty());
    assert!(configured.fail_open);
    assert_eq!(configured.guardrail_timeout, Duration::from_secs(30));
    assert!(!configured.streaming_end_of_stream_only);
    assert_eq!(configured.streaming_sampling_rate, 5);
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
        ("GRAYSWAN_REASONING_MODE".to_owned(), "HYBRID".to_owned()),
    ]);
    let source = GRAYSWAN_CONFIG.replace("      api_key: configured-key\n", "");
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_key.expose_secret(), "env-key");
    assert_eq!(configured.api_base, "https://example.test");
    assert!((configured.violation_threshold - 0.7).abs() < f64::EPSILON);
    assert_eq!(
        configured.reasoning_mode,
        Some(super::ReasoningMode::Hybrid)
    );
}

#[test]
fn grayswan_config_silently_drops_an_unsupported_reasoning_mode() {
    let source = format!("{GRAYSWAN_CONFIG}      reasoning_mode: verbose\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.reasoning_mode, None);
}

#[test]
fn grayswan_config_defaults_an_unsupported_on_flagged_action_to_monitor() {
    let source = format!("{GRAYSWAN_CONFIG}      on_flagged_action: yolo\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(
        configured.on_flagged_action,
        super::OnFlaggedAction::Monitor
    );
}

#[test]
fn grayswan_config_resolves_on_flagged_action_monitor_explicitly() {
    let source = format!("{GRAYSWAN_CONFIG}      on_flagged_action: monitor\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(
        configured.on_flagged_action,
        super::OnFlaggedAction::Monitor
    );
}

#[test]
fn grayswan_config_resolves_on_flagged_action_monitor_without_a_warning() {
    let (action, log) = logged(|| super::resolve_on_flagged_action(Some("monitor")));
    assert_eq!(action, super::OnFlaggedAction::Monitor);
    assert!(!log.contains("unsupported on_flagged_action"));
}

#[test]
fn grayswan_config_resolves_on_flagged_action_passthrough() {
    let source = format!("{GRAYSWAN_CONFIG}      on_flagged_action: passthrough\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(
        configured.on_flagged_action,
        super::OnFlaggedAction::Passthrough
    );
}

#[test]
fn grayswan_config_parses_reasoning_mode_off() {
    let source = format!("{GRAYSWAN_CONFIG}      reasoning_mode: off\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.reasoning_mode, Some(super::ReasoningMode::Off));
}

#[test]
fn grayswan_config_parses_reasoning_mode_thinking() {
    let source = format!("{GRAYSWAN_CONFIG}      reasoning_mode: thinking\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(
        configured.reasoning_mode,
        Some(super::ReasoningMode::Thinking)
    );
}

#[test]
fn grayswan_config_rejects_a_negative_guardrail_timeout() {
    let source = format!("{GRAYSWAN_CONFIG}      guardrail_timeout: -1\n");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, ConfigError::InvalidDuration { path, .. }
        if path == "guardrails.0.litellm_params.guardrail_timeout"));
}

#[test]
fn grayswan_config_logs_invalid_duration_category() {
    let source = format!("{GRAYSWAN_CONFIG}      guardrail_timeout: -1\n");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert_eq!(error_category(&error), "invalid_duration");
}

#[test]
fn grayswan_config_rejects_a_nan_guardrail_timeout() {
    let source = format!("{GRAYSWAN_CONFIG}      guardrail_timeout: .nan\n");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, ConfigError::InvalidDuration { path, .. }
        if path == "guardrails.0.litellm_params.guardrail_timeout"));
}

#[test]
fn grayswan_config_rejects_a_too_large_guardrail_timeout() {
    let source = format!("{GRAYSWAN_CONFIG}      guardrail_timeout: 1e300\n");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, ConfigError::InvalidDuration { path, .. }
        if path == "guardrails.0.litellm_params.guardrail_timeout"));
}

#[test]
fn grayswan_config_matches_on_flagged_action_case_insensitively() {
    let source = format!("{GRAYSWAN_CONFIG}      on_flagged_action: Block\n");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.on_flagged_action, super::OnFlaggedAction::Block);
}

#[test]
fn grayswan_config_explicit_values_win() {
    let lookup = BTreeMap::from([(
        "GRAYSWAN_API_BASE".to_owned(),
        "https://env.example".to_owned(),
    )]);
    let source = format!(
        "{GRAYSWAN_CONFIG}      api_base: https://configured.example\n      on_flagged_action: BLOCK\n      violation_threshold: \"0.9\"\n      categories:\n        jailbreak: block\n      policy_id: policy-1\n      fail_open: false\n      guardrail_timeout: 5\n      streaming_end_of_stream_only: true\n      streaming_sampling_rate: 2\n"
    );
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_base, "https://configured.example");
    assert_eq!(configured.on_flagged_action, super::OnFlaggedAction::Block);
    assert!((configured.violation_threshold - 0.9).abs() < f64::EPSILON);
    assert_eq!(
        configured.categories,
        BTreeMap::from([("jailbreak".to_owned(), "block".to_owned())])
    );
    assert_eq!(configured.policy_id, "policy-1");
    assert!(!configured.fail_open);
    assert_eq!(configured.guardrail_timeout, Duration::from_secs(5));
    assert!(configured.streaming_end_of_stream_only);
    assert_eq!(configured.streaming_sampling_rate, 2);
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
