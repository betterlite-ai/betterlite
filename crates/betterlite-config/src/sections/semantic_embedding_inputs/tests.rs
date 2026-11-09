#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::Section as _;
use crate::sections::semantic_embedding_inputs::{
    DEFAULT_TIMEOUT_SECONDS, EMBEDDING_TIMEOUT_ENV, Raw,
};

#[rstest]
#[case::absent(&[], DEFAULT_TIMEOUT_SECONDS)]
#[case::configured(&[(EMBEDDING_TIMEOUT_ENV, "2.5")], 2.5_f64)]
#[case::zero_is_kept(&[(EMBEDDING_TIMEOUT_ENV, "0")], 0.0_f64)]
#[case::negative_is_kept(&[(EMBEDDING_TIMEOUT_ENV, "-1.5")], -1.5_f64)]
#[case::exponent_is_kept(&[(EMBEDDING_TIMEOUT_ENV, "1e2")], 100.0_f64)]
#[case::whitespace_is_trimmed(&[(EMBEDDING_TIMEOUT_ENV, " 7.5 ")], 7.5_f64)]
#[case::empty_is_unset(&[(EMBEDDING_TIMEOUT_ENV, "")], DEFAULT_TIMEOUT_SECONDS)]
#[case::blank_is_unset(&[(EMBEDDING_TIMEOUT_ENV, "   ")], DEFAULT_TIMEOUT_SECONDS)]
fn timeout_reads_the_environment(#[case] variables: &[(&str, &str)], #[case] expected: f64) {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str("{}").unwrap();
    let settings = raw.convert(&environment).unwrap();
    assert_eq!(
        settings.embedding_timeout_seconds.to_bits(),
        expected.to_bits()
    );
}

#[rstest]
#[case::not_a_number("fast")]
#[case::python_underscore_spelling("1_0")]
#[case::trailing_text("5s")]
#[case::not_decimal_notation("nan")]
fn an_unparsable_timeout_names_the_variable(#[case] value: &str) {
    let environment = BTreeMap::from([(EMBEDDING_TIMEOUT_ENV.to_owned(), value.to_owned())]);
    let raw: Raw = serde_saphyr::from_str("{}").unwrap();
    let error = raw.convert(&environment).unwrap_err();
    assert!(error.to_string().contains(EMBEDDING_TIMEOUT_ENV));
    assert_eq!(error.category(), "environment");
}

#[test]
fn the_loaded_config_carries_the_timeout() {
    let lookup = BTreeMap::from([(EMBEDDING_TIMEOUT_ENV.to_owned(), "3.5".to_owned())]);
    let config = ConfigLoader { lookup }
        .load("litellm_settings: {}")
        .unwrap();
    assert_eq!(
        config
            .sections
            .semantic_embedding_inputs
            .embedding_timeout_seconds
            .to_bits(),
        3.5_f64.to_bits()
    );
}
