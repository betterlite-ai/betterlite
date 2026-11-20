#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use super::{DEFAULT_MAX_SIZE, DEFAULT_TTL_SECONDS, MAX_SIZE_VARIABLE, Settings, TTL_VARIABLE};
use crate::loader::{ConfigError, ConfigLoader};

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load("")?
        .sections
        .mcp_sso_assertion_cache)
}

fn assert_rejected(name: &str, value: &str) {
    assert!(matches!(
        load(&[(name, value)]),
        Err(ConfigError::Environment {
            source: EnvError::Number { name: failed }
        }) if failed == name
    ));
}

#[rstest]
fn uses_litellm_defaults() {
    assert_eq!(DEFAULT_TTL_SECONDS, 60);
    assert_eq!(DEFAULT_MAX_SIZE, 200);
    let settings = load(&[]).unwrap();
    assert_eq!(settings.ttl_seconds, DEFAULT_TTL_SECONDS);
    assert_eq!(settings.max_size, DEFAULT_MAX_SIZE);
    assert_eq!(Settings::default(), settings);
}

#[rstest]
fn reads_both_env_vars() {
    let settings = load(&[(TTL_VARIABLE, "45"), (MAX_SIZE_VARIABLE, "32")]).unwrap();
    assert_eq!(settings.ttl_seconds, 45);
    assert_eq!(settings.max_size, 32);
    let zero = load(&[(TTL_VARIABLE, "0"), (MAX_SIZE_VARIABLE, "0")]).unwrap();
    assert_eq!(zero.ttl_seconds, 0);
    assert_eq!(zero.max_size, 0);
    let ttl_only = load(&[(TTL_VARIABLE, "7")]).unwrap();
    assert_eq!(ttl_only.ttl_seconds, 7);
    assert_eq!(ttl_only.max_size, DEFAULT_MAX_SIZE);
    let size_only = load(&[(MAX_SIZE_VARIABLE, "3")]).unwrap();
    assert_eq!(size_only.ttl_seconds, DEFAULT_TTL_SECONDS);
    assert_eq!(size_only.max_size, 3);
    let blank = load(&[(TTL_VARIABLE, "  "), (MAX_SIZE_VARIABLE, "")]).unwrap();
    assert_eq!(blank.ttl_seconds, DEFAULT_TTL_SECONDS);
    assert_eq!(blank.max_size, DEFAULT_MAX_SIZE);
}

#[rstest]
fn rejects_a_non_number() {
    assert_rejected(TTL_VARIABLE, "not-a-number");
    assert_rejected(TTL_VARIABLE, "1.5");
    assert_rejected(TTL_VARIABLE, "-5");
    assert_rejected(MAX_SIZE_VARIABLE, "not-a-number");
    assert_rejected(MAX_SIZE_VARIABLE, "1.5");
    assert_rejected(MAX_SIZE_VARIABLE, "-1");
}
