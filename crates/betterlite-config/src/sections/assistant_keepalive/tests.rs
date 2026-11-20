#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::assistant_keepalive::{PingSetting, Settings};

const KEY: &str = "litellm_settings.sse_keepalive_ping_interval_seconds";

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .assistant_keepalive)
}

fn source(value: &str) -> String {
    format!("litellm_settings:\n  sse_keepalive_ping_interval_seconds: {value}")
}

#[test]
fn an_absent_key_gives_none() {
    let loaded = settings("litellm_settings: {}", &[]).expect("an empty section loads");
    assert_eq!(loaded.sse_keepalive_ping_interval_seconds, None);
}

#[rstest]
#[case::null("null")]
#[case::empty("")]
fn a_null_key_gives_none(#[case] value: &str) {
    let loaded = settings(&source(value), &[]).expect("a null value loads");
    assert_eq!(loaded.sse_keepalive_ping_interval_seconds, None);
}

#[rstest]
#[case::zero("0", 0.0_f64)]
#[case::negative("-1", -1.0_f64)]
#[case::integer("1", 1.0_f64)]
#[case::fraction("2.5", 2.5_f64)]
fn numbers_become_seconds(#[case] value: &str, #[case] expected: f64) {
    let loaded = settings(&source(value), &[]).expect("a number loads");
    assert_eq!(
        loaded.sse_keepalive_ping_interval_seconds,
        Some(PingSetting::Seconds(expected))
    );
}

#[rstest]
#[case::bad("bad", "bad")]
#[case::nan("nan", "nan")]
#[case::inf("inf", "inf")]
#[case::numeric_text("'1.5'", "1.5")]
fn strings_stay_text(#[case] value: &str, #[case] expected: &str) {
    let loaded = settings(&source(value), &[]).expect("a string loads");
    assert_eq!(
        loaded.sse_keepalive_ping_interval_seconds,
        Some(PingSetting::Text(expected.to_owned()))
    );
}

#[rstest]
#[case::boolean("true")]
#[case::list("[1, 2]")]
#[case::mapping("{seconds: 1}")]
fn values_outside_the_declared_type_name_the_key(#[case] value: &str) {
    match settings(&source(value), &[]) {
        Err(ConfigError::InvalidScalar { path, .. }) => assert_eq!(path, KEY),
        other => panic!("expected an invalid-scalar error, got {other:?}"),
    }
}

#[test]
fn an_environment_reference_is_resolved() {
    let loaded =
        settings(&source("os.environ/PING"), &[("PING", "2.5")]).expect("the reference resolves");
    assert_eq!(
        loaded.sse_keepalive_ping_interval_seconds,
        Some(PingSetting::Text("2.5".to_owned()))
    );
}

#[test]
fn a_missing_environment_reference_fails_the_load() {
    let error = settings(&source("os.environ/PING"), &[]).expect_err("the reference is missing");
    assert!(error.to_string().contains(KEY), "{error}");
}

#[test]
fn the_key_is_not_an_unsupported_key() {
    let loaded = settings(&source("2.5"), &[]).expect("the key is supported");
    assert_eq!(
        loaded.sse_keepalive_ping_interval_seconds,
        Some(PingSetting::Seconds(2.5_f64))
    );
}
