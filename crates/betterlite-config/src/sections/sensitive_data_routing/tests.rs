#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;

use super::{DEFAULT_TTL_SECONDS, Raw, Settings, TTL_ENV_VAR};
use crate::loader::ConfigError;
use crate::sections::Section;

fn settings(entries: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Raw::default().convert(&lookup)
}

#[test]
fn unset_variable_uses_the_litellm_default() {
    let parsed = settings(&[]).unwrap();
    assert_eq!(parsed.ttl_seconds, DEFAULT_TTL_SECONDS);
    assert_eq!(parsed.ttl_seconds, 3600);
}

#[test]
fn configured_variable_is_read() {
    let parsed = settings(&[(TTL_ENV_VAR, "7200")]).unwrap();
    assert_eq!(parsed.ttl_seconds, 7200);
}

#[test]
fn empty_variable_counts_as_unset() {
    let parsed = settings(&[(TTL_ENV_VAR, "")]).unwrap();
    assert_eq!(parsed.ttl_seconds, DEFAULT_TTL_SECONDS);
}

#[test]
fn spaces_around_the_value_are_trimmed() {
    let parsed = settings(&[(TTL_ENV_VAR, "  900  ")]).unwrap();
    assert_eq!(parsed.ttl_seconds, 900);
}

#[test]
fn zero_is_used_as_given() {
    let parsed = settings(&[(TTL_ENV_VAR, "0")]).unwrap();
    assert_eq!(parsed.ttl_seconds, 0);
}

#[test]
fn values_above_the_u32_range_are_read() {
    let parsed = settings(&[(TTL_ENV_VAR, "4294967296")]).unwrap();
    assert_eq!(parsed.ttl_seconds, 4_294_967_296);
}

#[test]
fn u64_max_is_used_as_given() {
    let parsed = settings(&[(TTL_ENV_VAR, "18446744073709551615")]).unwrap();
    assert_eq!(parsed.ttl_seconds, u64::MAX);
}

#[test]
fn values_above_u64_max_are_a_load_error_naming_the_variable() {
    let error = settings(&[(TTL_ENV_VAR, "18446744073709551616")]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name },
        } if name == TTL_ENV_VAR
    ));
}

#[test]
fn another_variable_is_not_mistaken_for_the_ttl() {
    let parsed = settings(&[("SENSITIVE_ROUTING_TTL", "12")]).unwrap();
    assert_eq!(parsed.ttl_seconds, DEFAULT_TTL_SECONDS);
}

#[test]
fn text_value_is_a_load_error_naming_the_variable() {
    let error = settings(&[(TTL_ENV_VAR, "soon")]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name },
        } if name == TTL_ENV_VAR
    ));
}

#[test]
fn negative_value_is_a_load_error_naming_the_variable() {
    let error = settings(&[(TTL_ENV_VAR, "-1")]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name },
        } if name == TTL_ENV_VAR
    ));
}

#[test]
fn fractional_value_is_a_load_error_naming_the_variable() {
    let error = settings(&[(TTL_ENV_VAR, "1.5")]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name },
        } if name == TTL_ENV_VAR
    ));
}

#[test]
fn default_settings_use_the_litellm_default() {
    let settings = Settings::default();
    assert_eq!(settings.ttl_seconds, DEFAULT_TTL_SECONDS);
}
