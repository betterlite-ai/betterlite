#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn loaded(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load("{}")?;
    Ok(config.sections.dynamic_rate_limit_health)
}

fn threshold(environment: &[(&str, &str)]) -> i64 {
    loaded(environment)
        .expect("the config loads")
        .error_threshold_per_minute
}

#[test]
fn defaults_to_one_without_the_variable() {
    assert_eq!(threshold(&[]), 1);
}

#[test]
fn reads_the_threshold_from_the_environment() {
    assert_eq!(
        threshold(&[("DYNAMIC_RATE_LIMIT_ERROR_THRESHOLD_PER_MINUTE", "4")]),
        4
    );
}

#[rstest]
#[case::zero("0")]
#[case::negative("-2")]
#[case::padded(" 7 ")]
fn keeps_a_plain_base_ten_integer(#[case] text: &str) {
    assert_eq!(
        threshold(&[("DYNAMIC_RATE_LIMIT_ERROR_THRESHOLD_PER_MINUTE", text)]),
        text.trim()
            .parse::<i64>()
            .expect("the case text is an integer")
    );
}

#[test]
fn an_empty_value_counts_as_unset() {
    assert_eq!(
        threshold(&[("DYNAMIC_RATE_LIMIT_ERROR_THRESHOLD_PER_MINUTE", "")]),
        1
    );
}

#[rstest]
#[case::words("two")]
#[case::decimal("1.5")]
#[case::underscore("1_0")]
fn rejects_a_value_that_is_not_a_plain_integer(#[case] text: &str) {
    let outcome = loaded(&[("DYNAMIC_RATE_LIMIT_ERROR_THRESHOLD_PER_MINUTE", text)]);
    assert!(matches!(outcome, Err(ConfigError::Environment { .. })));
}
