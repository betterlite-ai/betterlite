#![cfg(test)]

use super::{SettingsError, build_sentry_init_options, send_default_pii_from_env};
use std::collections::BTreeMap;

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn sentry_scrubbing_behavior_tests_behavior() {
    assert!(!send_default_pii_from_env(&env(&[])));
    assert!(send_default_pii_from_env(&env(&[(
        "SENTRY_SEND_DEFAULT_PII",
        " True "
    )])));
    assert!(!send_default_pii_from_env(&env(&[(
        "SENTRY_SEND_DEFAULT_PII",
        "yes"
    )])));
    assert!(!send_default_pii_from_env(&env(&[(
        "SENTRY_SEND_DEFAULT_PII",
        "FALSE"
    )])));
}

#[test]
fn defaults_apply_when_env_is_empty_or_blank() {
    let defaults = build_sentry_init_options(&env(&[])).unwrap();
    assert_eq!(defaults.dsn, None);
    assert!((defaults.traces_sample_rate - 1.0).abs() < f64::EPSILON);
    assert!((defaults.sample_rate - 1.0).abs() < f64::EPSILON);
    assert!(!defaults.send_default_pii);
    assert_eq!(defaults.environment, "production");
    assert_eq!(defaults.custom_key_minimum, 16);
    assert_eq!(defaults.generated_key_bytes, 16);

    let blank_rate = build_sentry_init_options(&env(&[("SENTRY_API_TRACE_RATE", "")])).unwrap();
    assert!((blank_rate.traces_sample_rate - 1.0).abs() < f64::EPSILON);
}

#[test]
fn key_lengths_are_read_from_their_own_env_vars() {
    let configured = build_sentry_init_options(&env(&[
        ("MINIMUM_CUSTOM_KEY_LENGTH", "8"),
        ("LENGTH_OF_LITELLM_GENERATED_KEY", "32"),
    ]))
    .unwrap();
    assert_eq!(configured.custom_key_minimum, 8);
    assert_eq!(configured.generated_key_bytes, 32);
}

#[test]
fn malformed_key_lengths_fail_with_typed_errors() {
    let minimum_error =
        build_sentry_init_options(&env(&[("MINIMUM_CUSTOM_KEY_LENGTH", "not-a-number")]))
            .unwrap_err();
    assert!(matches!(
        minimum_error,
        SettingsError::InvalidCustomKeyMinimum { .. }
    ));

    let generated_error =
        build_sentry_init_options(&env(&[("LENGTH_OF_LITELLM_GENERATED_KEY", "not-a-number")]))
            .unwrap_err();
    assert!(matches!(
        generated_error,
        SettingsError::InvalidGeneratedKeyLength { .. }
    ));
}

#[test]
fn configured_env_values_are_carried_through() {
    let configured = build_sentry_init_options(&env(&[
        ("SENTRY_DSN", "https://example/1"),
        ("SENTRY_API_TRACE_RATE", "0.5"),
        ("SENTRY_API_SAMPLE_RATE", "0.25"),
        ("SENTRY_ENVIRONMENT", "staging"),
        ("SENTRY_SEND_DEFAULT_PII", "true"),
    ]))
    .unwrap();
    assert_eq!(configured.dsn, Some("https://example/1".to_owned()));
    assert!((configured.traces_sample_rate - 0.5).abs() < f64::EPSILON);
    assert!((configured.sample_rate - 0.25).abs() < f64::EPSILON);
    assert_eq!(configured.environment, "staging");
    assert!(configured.send_default_pii);
}

#[test]
fn malformed_rate_values_fail_with_typed_errors() {
    let trace_error =
        build_sentry_init_options(&env(&[("SENTRY_API_TRACE_RATE", "not-a-float")])).unwrap_err();
    assert!(matches!(
        trace_error,
        SettingsError::InvalidTraceRate { .. }
    ));

    let sample_error =
        build_sentry_init_options(&env(&[("SENTRY_API_SAMPLE_RATE", "not-a-float")])).unwrap_err();
    assert!(matches!(
        sample_error,
        SettingsError::InvalidSampleRate { .. }
    ));
}

#[test]
fn event_scrubber_denylist_folds_in_pii_names_only_without_send_default_pii() {
    let without_pii = build_sentry_init_options(&env(&[])).unwrap().event_scrubber;
    assert!(without_pii.denylist.contains(&"api_key".to_owned()));
    assert!(without_pii.denylist.contains(&"user_email".to_owned()));
    assert!(without_pii.recursive);

    let with_pii = build_sentry_init_options(&env(&[("SENTRY_SEND_DEFAULT_PII", "true")]))
        .unwrap()
        .event_scrubber;
    assert!(with_pii.denylist.contains(&"api_key".to_owned()));
    assert!(!with_pii.denylist.contains(&"user_email".to_owned()));

    assert!(
        without_pii
            .denylist
            .iter()
            .all(|name| name == &name.to_lowercase())
    );
}
