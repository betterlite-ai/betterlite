#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::gemini_cache_create::Settings;

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .expect("the config loads")
        .sections
        .gemini_cache_create
}

#[rstest]
#[case::whole_number("512", Some(512))]
#[case::zero("0", Some(0))]
#[case::largest_signed("9223372036854775807", Some(0x7FFF_FFFF_FFFF_FFFF))]
#[case::surrounding_spaces(" 2048 ", Some(2048))]
#[case::plus_sign("+7", Some(7))]
#[case::negative("-1", Some(0))]
fn usable_values_become_the_override(#[case] value: &str, #[case] expected: Option<u64>) {
    assert_eq!(
        settings(&[("MINIMUM_PROMPT_CACHE_TOKEN_COUNT", value)]).minimum_token_override,
        expected
    );
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::text("many")]
#[case::fraction("1.5")]
#[case::exponent("1e3")]
#[case::beyond_signed("9223372036854775808")]
fn malformed_values_fall_back_to_the_model_minimum(#[case] value: &str) {
    assert_eq!(
        settings(&[("MINIMUM_PROMPT_CACHE_TOKEN_COUNT", value)]).minimum_token_override,
        None
    );
}

#[test]
fn an_unset_variable_leaves_the_model_minimum() {
    assert_eq!(
        settings(&[("SOME_OTHER_VARIABLE", "512")]).minimum_token_override,
        None
    );
}

#[test]
fn the_override_comes_from_the_injected_environment() {
    assert_eq!(
        settings(&[("MINIMUM_PROMPT_CACHE_TOKEN_COUNT", "7")]).minimum_token_override,
        Some(7)
    );
}

#[test]
fn an_unrelated_litellm_key_stays_unsupported() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load("litellm_settings:\n  unrelated_key: 1\n")
        .expect_err("the unknown key is refused");
    assert!(matches!(error, ConfigError::Unsupported { .. }));
}
