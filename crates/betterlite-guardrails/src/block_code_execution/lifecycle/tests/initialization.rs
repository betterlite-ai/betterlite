use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::content_filter_records::BlockCodeAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_guardrails::block_code_execution::initializer::{
    InitializerError, initialize_guardrail,
};
use rstest::rstest;

use crate::block_code_execution::support::{
    FakeRegistry, guardrail_entry, initializer_error, litellm_params,
};

#[test]
fn an_empty_name_is_rejected() {
    let params = litellm_params(r#"{"guardrail": "block_code_execution", "mode": "pre_call"}"#);
    let entry = guardrail_entry("", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(error, InitializerError::MissingGuardrailName));
    assert_eq!(
        error.to_string(),
        "Block Code Execution guardrail requires a guardrail_name"
    );
    assert!(registry.callbacks.is_empty());
}

#[test]
fn the_full_configuration_reaches_the_guardrail() {
    let params = litellm_params(
        r#"{
            "guardrail": "block_code_execution",
            "mode": ["pre_call", "post_call"],
            "blocked_languages": ["python", "bash"],
            "action": "mask",
            "confidence_threshold": 0.9,
            "detect_execution_intent": false,
            "default_on": true
        }"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let instance = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry)
        .expect("initializes");
    let settings = &instance.base.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("mine"));
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "post_call".to_owned()
        ]))
    );
    assert!(settings.default_on);
    assert_eq!(
        instance.blocked_languages,
        Some(vec!["python".to_owned(), "bash".to_owned()])
    );
    assert!(!instance.block_all);
    assert_eq!(instance.scanner.action, BlockCodeAction::Mask);
    assert_eq!(
        instance.scanner.detector.confidence_threshold.to_bits(),
        0.9_f64.to_bits()
    );
    assert!(!instance.scanner.detect_execution_intent);
}

#[test]
fn defaults_apply_when_the_settings_are_absent() {
    let params = litellm_params(r#"{"guardrail": "block_code_execution", "mode": "pre_call"}"#);
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let instance = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry)
        .expect("initializes");
    assert!(instance.blocked_languages.is_none());
    assert!(instance.block_all);
    assert_eq!(instance.scanner.action, BlockCodeAction::Block);
    assert_eq!(
        instance.scanner.detector.confidence_threshold.to_bits(),
        0.5_f64.to_bits()
    );
    assert!(instance.scanner.detect_execution_intent);
    assert!(!instance.base.settings.default_on);
    assert_eq!(
        instance.base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
}

#[test]
fn the_instance_is_registered_as_a_callback() {
    let params = litellm_params(r#"{"guardrail": "block_code_execution", "mode": "pre_call"}"#);
    let entry = guardrail_entry("registered", params);
    let mut registry = FakeRegistry::default();
    let instance = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry)
        .expect("initializes");
    assert_eq!(registry.callbacks.len(), 1);
    assert!(Arc::ptr_eq(&registry.callbacks[0], &instance));
}

#[test]
fn a_tagged_mode_is_rejected() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": {"tags": {"a": "pre_call"}}}"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == "mode"
    ));
}

#[test]
fn a_non_boolean_intent_flag_is_rejected() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "detect_execution_intent": "true"}"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == "detect_execution_intent"
    ));
}

#[test]
fn a_non_list_blocked_languages_is_rejected() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "blocked_languages": "python"}"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == "blocked_languages"
    ));
}

#[test]
fn a_numeric_string_confidence_threshold_is_read() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "confidence_threshold": "0.7"}"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let instance = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry)
        .expect("a numeric string is read");
    assert_eq!(
        instance.scanner.detector.confidence_threshold.to_bits(),
        0.7_f64.to_bits()
    );
}

#[rstest]
#[case::above_the_range("2.5")]
#[case::below_the_range("-0.1")]
fn an_out_of_range_confidence_threshold_is_rejected(#[case] threshold: &str) {
    let params = litellm_params(&format!(
        r#"{{"guardrail": "block_code_execution", "mode": "pre_call", "confidence_threshold": {threshold}}}"#
    ));
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == "confidence_threshold"
    ));
}

#[test]
fn a_not_a_number_confidence_threshold_is_rejected() {
    let fields = BTreeMap::from([
        (
            "guardrail".to_owned(),
            GuardrailValue::Text("block_code_execution".to_owned()),
        ),
        (
            "mode".to_owned(),
            GuardrailValue::Text("pre_call".to_owned()),
        ),
        (
            "confidence_threshold".to_owned(),
            GuardrailValue::Decimal(f64::NAN),
        ),
    ]);
    let params = LitellmParams::try_from(OpaqueParams(fields)).expect("params");
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == "confidence_threshold"
    ));
}

#[rstest]
#[case::action_null("action")]
#[case::threshold_null("confidence_threshold")]
#[case::intent_null("detect_execution_intent")]
fn a_null_for_a_required_parameter_is_rejected(#[case] key: &str) {
    let params = litellm_params(&format!(
        r#"{{"guardrail": "block_code_execution", "mode": "pre_call", "{key}": null}}"#
    ));
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == key
    ));
}

#[test]
fn an_empty_confidence_threshold_is_rejected() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "confidence_threshold": ""}"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == "confidence_threshold"
    ));
}

#[test]
fn a_null_blocked_languages_and_default_on_keep_their_defaults() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "blocked_languages": null, "default_on": null}"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let instance = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry)
        .expect("nulls on optional fields take the defaults");
    assert!(instance.blocked_languages.is_none());
    assert!(instance.block_all);
    assert!(!instance.base.settings.default_on);
}

#[test]
fn an_unreadable_confidence_threshold_is_rejected() {
    let params = litellm_params(
        r#"{"guardrail": "block_code_execution", "mode": "pre_call", "confidence_threshold": "high"}"#,
    );
    let entry = guardrail_entry("mine", params);
    let mut registry = FakeRegistry::default();
    let outcome = initialize_guardrail(&entry.litellm_params, &entry, true, &mut registry);
    let error = initializer_error(outcome);
    assert!(matches!(
        &error,
        InitializerError::InvalidParam { field } if field == "confidence_threshold"
    ));
}
