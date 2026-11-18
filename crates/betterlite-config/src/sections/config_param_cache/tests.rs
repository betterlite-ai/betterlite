#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

use super::{DEFAULT_TTL_SECONDS, Settings, TTL_VARIABLE};

fn settings(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []\n")
        .map(|config| config.sections.config_param_cache)
}

#[test]
fn ttl_defaults_to_sixty_seconds() {
    let settings = settings(&[]).expect("the default loads");
    assert_eq!(
        settings.ttl_seconds.to_bits(),
        DEFAULT_TTL_SECONDS.to_bits()
    );
    assert_eq!(settings.ttl_seconds.to_bits(), 60.0_f64.to_bits());
}

#[test]
fn ttl_reads_the_environment() {
    let settings = settings(&[(TTL_VARIABLE, "15")]).expect("the value loads");
    assert_eq!(settings.ttl_seconds.to_bits(), 15.0_f64.to_bits());
}

#[test]
fn ttl_keeps_an_explicit_zero() {
    let settings = settings(&[(TTL_VARIABLE, "0")]).expect("an explicit zero loads");
    assert_eq!(settings.ttl_seconds.to_bits(), 0.0_f64.to_bits());
}

#[test]
fn empty_ttl_counts_as_unset() {
    let settings = settings(&[(TTL_VARIABLE, "  ")]).expect("an empty value falls back");
    assert_eq!(
        settings.ttl_seconds.to_bits(),
        DEFAULT_TTL_SECONDS.to_bits()
    );
}

#[test]
fn invalid_ttl_names_the_variable() {
    let error = settings(&[(TTL_VARIABLE, "sixty")]).expect_err("a word is not a number");
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: betterlite_env::EnvError::Number { name },
        } if name == TTL_VARIABLE
    ));
}
