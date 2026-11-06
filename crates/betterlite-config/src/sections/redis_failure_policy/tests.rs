#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

fn lookup(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = lookup(environment);
    ConfigLoader { lookup }
        .load("{}")
        .expect("the config should load")
        .sections
        .redis_failure_policy
}

fn error(environment: &[(&str, &str)]) -> ConfigError {
    ConfigLoader {
        lookup: lookup(environment),
    }
    .load("{}")
    .expect_err("the environment should be rejected")
}

#[test]
fn defaults_match_the_pinned_constants() {
    let settings = settings(&[]);
    assert!(settings.enabled);
    assert_eq!(settings.failure_threshold, 5);
    assert_eq!(settings.recovery_timeout, Duration::from_secs(60));
    assert_eq!(settings.timeout_min_duration, Duration::from_secs(5));
    assert_eq!(settings.timeout_log_interval, Duration::from_secs(5));
}

#[rstest]
#[case::disabled("REDIS_CIRCUIT_BREAKER_ENABLED", "false")]
#[case::enabled_spelled_as_a_number("REDIS_CIRCUIT_BREAKER_ENABLED", "1")]
#[case::threshold("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD", "9")]
#[case::recovery_timeout("REDIS_CIRCUIT_BREAKER_RECOVERY_TIMEOUT", "30")]
#[case::timeout_min_duration("REDIS_CIRCUIT_BREAKER_TIMEOUT_MIN_DURATION", "2.5")]
#[case::timeout_log_interval("REDIS_TIMEOUT_LOG_INTERVAL", "0.25")]
fn reads_each_environment_override(#[case] name: &str, #[case] value: &str) {
    let settings = settings(&[(name, value)]);
    let defaults = Settings::default();
    let fields = [
        settings.enabled != defaults.enabled,
        settings.failure_threshold != defaults.failure_threshold,
        settings.recovery_timeout != defaults.recovery_timeout,
        settings.timeout_min_duration != defaults.timeout_min_duration,
        settings.timeout_log_interval != defaults.timeout_log_interval,
    ];
    assert_eq!(fields.iter().filter(|differs| **differs).count(), 1);
}

#[test]
fn reads_the_values_into_the_policy_fields() {
    let settings = settings(&[
        ("REDIS_CIRCUIT_BREAKER_ENABLED", "off"),
        ("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD", "9"),
        ("REDIS_CIRCUIT_BREAKER_RECOVERY_TIMEOUT", "30"),
        ("REDIS_CIRCUIT_BREAKER_TIMEOUT_MIN_DURATION", "2.5"),
        ("REDIS_TIMEOUT_LOG_INTERVAL", "0.25"),
    ]);
    assert!(!settings.enabled);
    assert_eq!(settings.failure_threshold, 9);
    assert_eq!(settings.recovery_timeout, Duration::from_secs(30));
    assert_eq!(settings.timeout_min_duration, Duration::from_millis(2_500));
    assert_eq!(settings.timeout_log_interval, Duration::from_millis(250));
}

#[rstest]
#[case::enabled("REDIS_CIRCUIT_BREAKER_ENABLED")]
#[case::threshold("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD")]
#[case::recovery_timeout("REDIS_CIRCUIT_BREAKER_RECOVERY_TIMEOUT")]
#[case::timeout_min_duration("REDIS_CIRCUIT_BREAKER_TIMEOUT_MIN_DURATION")]
#[case::timeout_log_interval("REDIS_TIMEOUT_LOG_INTERVAL")]
fn an_empty_value_keeps_the_default(#[case] name: &str) {
    assert_eq!(settings(&[(name, "")]), Settings::default());
}

#[rstest]
#[case::enabled("REDIS_CIRCUIT_BREAKER_ENABLED", "maybe")]
#[case::threshold("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD", "five")]
#[case::negative_threshold("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD", "-1")]
#[case::recovery_timeout("REDIS_CIRCUIT_BREAKER_RECOVERY_TIMEOUT", "soon")]
#[case::negative_recovery_timeout("REDIS_CIRCUIT_BREAKER_RECOVERY_TIMEOUT", "-1")]
#[case::timeout_min_duration("REDIS_CIRCUIT_BREAKER_TIMEOUT_MIN_DURATION", "soon")]
#[case::negative_timeout_min_duration("REDIS_CIRCUIT_BREAKER_TIMEOUT_MIN_DURATION", "-1.5")]
#[case::timeout_log_interval("REDIS_TIMEOUT_LOG_INTERVAL", "soon")]
fn a_malformed_value_is_an_environment_error(#[case] name: &str, #[case] value: &str) {
    assert!(matches!(
        error(&[(name, value)]),
        ConfigError::Environment { source }
            if source == betterlite_env::EnvError::Number { name: name.to_owned() }
                || source == betterlite_env::EnvError::Bool { name: name.to_owned() }
    ));
}

#[test]
fn the_yaml_key_is_not_a_configuration_surface() {
    let lookup = BTreeMap::<String, String>::new();
    let error = ConfigLoader { lookup }
        .load("REDIS_CIRCUIT_BREAKER_ENABLED: true")
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Unsupported { paths } if paths == ["REDIS_CIRCUIT_BREAKER_ENABLED"]
    ));
}
