#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{ACCESS_TTL_VARIABLE, Settings};

const MANAGEMENT_TTL_VARIABLE: &str = "DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL";

fn settings(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []\n")
        .map(|config| config.sections.auth_invalidation)
}

#[test]
fn access_ttl_default_600() {
    let settings = settings(&[]).expect("the defaults load");
    assert_eq!(settings.access_group_ttl.to_bits(), 600.0_f64.to_bits());
}

#[test]
fn access_ttl_env_override() {
    let settings = settings(&[(ACCESS_TTL_VARIABLE, "120")]).expect("the override loads");
    assert_eq!(settings.access_group_ttl.to_bits(), 120.0_f64.to_bits());
}

#[rstest]
#[case::word("soon")]
#[case::fraction("1.5")]
#[case::empty("")]
#[case::blank("  ")]
fn access_ttl_bad_or_empty_stops_startup(#[case] value: &str) {
    let error = settings(&[(ACCESS_TTL_VARIABLE, value)])
        .expect_err("LiteLLM parses this value at import, so a bad one stops startup");
    assert!(error.to_string().contains(ACCESS_TTL_VARIABLE), "{error}");
}

#[test]
fn access_ttl_zero_negative_not_clamped() {
    let zero = settings(&[(ACCESS_TTL_VARIABLE, "0")]).expect("zero is accepted");
    assert_eq!(zero.access_group_ttl.to_bits(), 0.0_f64.to_bits());
    let negative = settings(&[(ACCESS_TTL_VARIABLE, "-5")]).expect("a negative value is accepted");
    assert_eq!(negative.access_group_ttl.to_bits(), (-5.0_f64).to_bits());
}

#[test]
fn management_ttl_does_not_override_access_ttl() {
    let independent = settings(&[(MANAGEMENT_TTL_VARIABLE, "45")])
        .expect("the management ttl alone leaves the access ttl at its default");
    assert_eq!(independent.access_group_ttl.to_bits(), 600.0_f64.to_bits());
    let both = settings(&[
        (MANAGEMENT_TTL_VARIABLE, "45"),
        (ACCESS_TTL_VARIABLE, "180"),
    ])
    .expect("both values load");
    assert_eq!(both.access_group_ttl.to_bits(), 180.0_f64.to_bits());
}
