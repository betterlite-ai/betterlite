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
#[case::not_a_number("4k")]
#[case::fraction("1.5")]
fn loads_settings_rejects_invalid_values(#[case] value: &str) {
    let error = load(&[(VARIABLE, value)]).expect_err("the value is not an integer");
    assert!(error.to_string().contains(VARIABLE));
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
    let lookup = BTreeMap::from([(VARIABLE.to_owned(), String::from("8192"))]);
    let config = ConfigLoader { lookup }
        .load("environment_variables:\n  DEFAULT_MAX_TOKENS: \"1234\"\n")
        .expect("the variable is a known key");
    assert_eq!(config.litellm_settings.default_max_tokens, 8192);
}
