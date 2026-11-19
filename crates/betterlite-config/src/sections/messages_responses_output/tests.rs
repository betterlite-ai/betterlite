#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

const VARIABLE: &str = "DEFAULT_MAX_TOKENS";
const DEFAULT_TOKENS: i64 = 4096;

fn load(environment: &[(&str, &str)]) -> Result<i64, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .map(|config| config.litellm_settings.default_max_tokens)
}

#[rstest]
#[case::unset_defaults(&[], DEFAULT_TOKENS)]
#[case::explicit_override(&[(VARIABLE, "8192")], 8192)]
#[case::surrounding_space(&[(VARIABLE, " 123 ")], 123)]
#[case::explicit_zero_is_used(&[(VARIABLE, "0")], 0)]
#[case::negative_is_used(&[(VARIABLE, "-5")], -5)]
#[case::empty_counts_as_unset(&[(VARIABLE, "")], DEFAULT_TOKENS)]
fn loads_settings(#[case] environment: &[(&str, &str)], #[case] expected: i64) {
    assert_eq!(load(environment).expect("the variable loads"), expected);
}

#[rstest]
#[case::not_a_number("4k", "DEFAULT_MAX_TOKENS")]
#[case::fraction("1.5", "DEFAULT_MAX_TOKENS")]
fn loads_settings_rejects_invalid_values(#[case] value: &str, #[case] expected: &str) {
    let error = load(&[(VARIABLE, value)]).expect_err("the value is not an integer");
    assert!(error.to_string().contains(expected));
}

#[test]
fn loads_settings_accepts_the_litellm_settings_table() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  num_retries: 2\n")
    .expect("the table loads");
    assert_eq!(config.litellm_settings.default_max_tokens, DEFAULT_TOKENS);
}

#[test]
fn loads_settings_reports_no_unsupported_key() {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let result = loader.load("general_settings:\n  master_key: sk-1\n");
    assert!(!matches!(result, Err(ConfigError::Unsupported { .. })));
}
