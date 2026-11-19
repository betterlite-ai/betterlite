#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.auth_prefetch)
}

#[test]
fn org_ttl_default_five() {
    assert_eq!(
        load(&[]).unwrap().organization_ttl.to_bits(),
        5.0_f64.to_bits()
    );
}

#[test]
fn org_ttl_env_override() {
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", "45")])
            .unwrap()
            .organization_ttl
            .to_bits(),
        45.0_f64.to_bits()
    );
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", " 7 ")])
            .unwrap()
            .organization_ttl
            .to_bits(),
        7.0_f64.to_bits()
    );
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", "-11")])
            .unwrap()
            .organization_ttl
            .to_bits(),
        (-11.0_f64).to_bits()
    );
}

#[rstest]
#[case::empty("")]
#[case::blank("  ")]
#[case::nonnumeric("soon")]
#[case::fractional("5.5")]
fn org_ttl_bad_or_empty_fails_startup(#[case] value: &str) {
    match load(&[("DEFAULT_IN_MEMORY_TTL", value)]) {
        Err(ConfigError::InvalidInteger { path, .. }) => assert_eq!(path, "DEFAULT_IN_MEMORY_TTL"),
        other => panic!("{value:?} should fail startup: {other:?}"),
    }
}

#[test]
fn org_ttl_zero_negative_kept() {
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", "0")])
            .unwrap()
            .organization_ttl
            .to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", "-5")])
            .unwrap()
            .organization_ttl
            .to_bits(),
        (-5.0_f64).to_bits()
    );
}

#[test]
fn org_membership_registry_ttls_independent() {
    let lookup = BTreeMap::from([
        ("DEFAULT_IN_MEMORY_TTL".to_owned(), "9".to_owned()),
        (
            "DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL".to_owned(),
            "75".to_owned(),
        ),
    ]);
    let sections = ConfigLoader { lookup }.load("").unwrap().sections;
    assert_eq!(
        sections.auth_prefetch.organization_ttl.to_bits(),
        9.0_f64.to_bits()
    );
    assert_eq!(
        sections.auth_cache.management_object_ttl.to_bits(),
        75.0_f64.to_bits()
    );
    let defaults = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap()
    .sections;
    assert_eq!(
        defaults.auth_prefetch.organization_ttl.to_bits(),
        5.0_f64.to_bits()
    );
    assert_eq!(
        defaults.auth_cache.management_object_ttl.to_bits(),
        60.0_f64.to_bits()
    );
}
