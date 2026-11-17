#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::prompt_affinity::Settings;

fn settings(source: &str) -> Settings {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .prompt_affinity
}

fn settings_with(variables: &[(&str, &str)], source: &str) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .prompt_affinity
}

#[rstest]
#[case::the_check_named(
    "router_settings:\n  optional_pre_call_checks:\n    - prompt_caching\n",
    true
)]
#[case::the_check_among_others(
    "router_settings:\n  optional_pre_call_checks:\n    - router_budget_limiting\n    - prompt_caching\n",
    true
)]
#[case::another_check_alone(
    "router_settings:\n  optional_pre_call_checks:\n    - router_budget_limiting\n",
    false
)]
#[case::an_empty_list("router_settings:\n  optional_pre_call_checks: []\n", false)]
#[case::a_different_spelling(
    "router_settings:\n  optional_pre_call_checks:\n    - Prompt_Caching\n",
    false
)]
#[case::the_key_absent("", false)]
fn the_configured_list_selects_the_check(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(settings(source).prompt_caching, expected);
}

#[test]
fn the_base64_limit_comes_from_the_injected_environment() {
    let configured = settings_with(&[("MAX_BASE64_LENGTH_FOR_LOGGING", "128")], "");
    assert_eq!(configured.maximum_base64_length, 128);
}

#[test]
fn an_unset_base64_limit_keeps_the_placeholder_default() {
    let configured = settings_with(&[("SOME_OTHER_VARIABLE", "128")], "");
    assert_eq!(configured.maximum_base64_length, 64);
}

// litellm.constants.MAX_BASE64_LENGTH_FOR_LOGGING is int(os.getenv(...)), so a value that is not a number stops the load
#[test]
fn a_base64_limit_that_is_not_a_number_stops_the_load() {
    let lookup: BTreeMap<String, String> = [(
        "MAX_BASE64_LENGTH_FOR_LOGGING".to_owned(),
        "many".to_owned(),
    )]
    .into_iter()
    .collect();
    assert!(ConfigLoader { lookup }.load("").is_err());
}
