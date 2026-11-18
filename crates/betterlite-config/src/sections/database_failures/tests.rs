#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

const CONCURRENCY: &str = "PROXY_DB_LOOKUP_MAX_CONCURRENCY";
const DEADLINE: &str = "PROXY_DB_LOOKUP_DEADLINE_SECONDS";

fn load(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.database_failures)
}

fn concurrency(value: &str) -> Result<usize, ConfigError> {
    load(&[(CONCURRENCY, value)]).map(|settings| settings.max_concurrency)
}

fn deadline(value: &str) -> Result<f64, ConfigError> {
    load(&[(DEADLINE, value)]).map(|settings| settings.deadline_seconds)
}

fn assert_invalid_integer(result: &Result<usize, ConfigError>, value: &str) {
    match result {
        Err(ConfigError::InvalidInteger { path, .. }) => assert_eq!(path, CONCURRENCY),
        other => panic!("{value:?} should stop startup: {other:?}"),
    }
}

fn assert_invalid_float(result: &Result<f64, ConfigError>, value: &str) {
    match result {
        Err(ConfigError::InvalidFloat { path, .. }) => assert_eq!(path, DEADLINE),
        other => panic!("{value:?} should stop startup: {other:?}"),
    }
}

#[test]
fn lookup_defaults() {
    let settings = load(&[]).unwrap();
    assert_eq!(settings.max_concurrency, 25);
    assert_eq!(settings.deadline_seconds.to_bits(), 10.0_f64.to_bits());
}

#[test]
fn explicit_values_override() {
    assert_eq!(concurrency("8").unwrap(), 8);
    assert_eq!(concurrency(" 12 ").unwrap(), 12);
    assert_eq!(concurrency("+25").unwrap(), 25);
    assert_eq!(deadline("2.5").unwrap().to_bits(), 2.5_f64.to_bits());
    assert_eq!(deadline("0.05").unwrap().to_bits(), 0.05_f64.to_bits());
}

#[test]
fn concurrency_floor_one() {
    assert_eq!(concurrency("0").unwrap(), 1);
    assert_eq!(concurrency("-5").unwrap(), 1);
    assert_eq!(concurrency("-9223372036854775808").unwrap(), 1);
}

#[test]
fn deadline_floor_point_one() {
    assert_eq!(deadline("0").unwrap().to_bits(), 0.1_f64.to_bits());
    assert_eq!(deadline("-3").unwrap().to_bits(), 0.1_f64.to_bits());
    assert_eq!(deadline("-0.0").unwrap().to_bits(), 0.1_f64.to_bits());
}

#[test]
fn invalid_integer_stops_startup() {
    assert_invalid_integer(&concurrency("soon"), "soon");
    assert_invalid_integer(&concurrency("2.5"), "2.5");
    assert_invalid_integer(&concurrency("  "), "  ");
    assert_invalid_integer(&concurrency(""), "");
}

#[test]
fn invalid_float_stops_startup() {
    assert_invalid_float(&deadline("soon"), "soon");
    assert_invalid_float(&deadline("1.5.5"), "1.5.5");
    assert_invalid_float(&deadline("  "), "  ");
}

#[test]
fn explicit_empty_env_stops_startup() {
    assert_invalid_integer(&concurrency(""), "");
    assert_invalid_float(&deadline(""), "");
}

#[test]
fn nonfinite_deadline_semantics() {
    assert_eq!(deadline("nan").unwrap().to_bits(), 0.1_f64.to_bits());
    assert_eq!(deadline("-inf").unwrap().to_bits(), 0.1_f64.to_bits());
    assert_eq!(deadline("inf").unwrap().to_bits(), f64::INFINITY.to_bits());
    assert_eq!(
        deadline("+infinity").unwrap().to_bits(),
        f64::INFINITY.to_bits()
    );
}
