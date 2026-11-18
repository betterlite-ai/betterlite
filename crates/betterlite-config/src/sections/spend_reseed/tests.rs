#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

const LOCKS: &str = "SPEND_COUNTER_RESEED_LOCKS_MAX_SIZE";

fn load(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.spend_reseed)
}

fn locks(value: &str) -> Result<i64, ConfigError> {
    load(&[(LOCKS, value)]).map(|settings| settings.locks_max_size)
}

#[test]
fn reseed_lock_default10000() {
    assert_eq!(load(&[]).unwrap().locks_max_size, 10_000);
    assert_eq!(locks("10000").unwrap(), 10_000);
}

#[rstest]
#[case::text("soon")]
#[case::decimal("2.5")]
#[case::empty("")]
#[case::spaces("  ")]
#[case::separated("10 000")]
#[test]
fn invalid_integer_stops_startup(#[case] value: &str) {
    match locks(value) {
        Err(ConfigError::InvalidInteger { path, .. }) => assert_eq!(path, LOCKS, "{value:?}"),
        other => panic!("{value:?} must stop startup: {other:?}"),
    }
}

#[test]
fn zero_negative_lock_bound_consumer() {
    // the loader hands the bound to the lock registry unchanged, which then evicts on every insert
    assert_eq!(locks("0").unwrap(), 0);
    assert_eq!(locks("-1").unwrap(), -1);
    assert_eq!(locks("-9223372036854775808").unwrap(), i64::MIN);
    assert_eq!(locks(" +7 ").unwrap(), 7);
}
