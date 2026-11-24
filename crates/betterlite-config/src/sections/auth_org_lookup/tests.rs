#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.auth_org_lookup)
}

#[test]
fn ttl_default_five() {
    assert_eq!(
        load(&[]).unwrap().in_memory_ttl.to_bits(),
        5.0_f64.to_bits()
    );
}

#[test]
fn ttl_integer_override() {
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", "45")])
            .unwrap()
            .in_memory_ttl
            .to_bits(),
        45.0_f64.to_bits()
    );
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", " 7 ")])
            .unwrap()
            .in_memory_ttl
            .to_bits(),
        7.0_f64.to_bits()
    );
}

#[rstest]
#[case::empty("")]
#[case::blank("  ")]
#[case::nonnumeric("soon")]
#[case::fractional("5.5")]
fn ttl_bad_integer_stops_startup(#[case] value: &str) {
    match load(&[("DEFAULT_IN_MEMORY_TTL", value)]) {
        Err(ConfigError::InvalidInteger { path, .. }) => assert_eq!(path, "DEFAULT_IN_MEMORY_TTL"),
        other => panic!("{value:?} should fail startup: {other:?}"),
    }
}

#[test]
fn ttl_zero_negative_passthrough() {
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", "0")])
            .unwrap()
            .in_memory_ttl
            .to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        load(&[("DEFAULT_IN_MEMORY_TTL", "-5")])
            .unwrap()
            .in_memory_ttl
            .to_bits(),
        (-5.0_f64).to_bits()
    );
}
