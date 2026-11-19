#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::model_rate_limits::Settings;

fn settings(source: &str) -> Settings {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .model_rate_limits
}

#[rstest]
#[case::the_check_named(
    "router_settings:\n  optional_pre_call_checks:\n    - enforce_model_rate_limits\n",
    true
)]
#[case::the_check_among_others(
    "router_settings:\n  optional_pre_call_checks:\n    - router_budget_limiting\n    - enforce_model_rate_limits\n",
    true
)]
#[case::another_check_alone(
    "router_settings:\n  optional_pre_call_checks:\n    - prompt_caching\n",
    false
)]
#[case::an_empty_list("router_settings:\n  optional_pre_call_checks: []\n", false)]
#[case::a_different_spelling(
    "router_settings:\n  optional_pre_call_checks:\n    - Enforce_Model_Rate_Limits\n",
    false
)]
#[case::the_key_absent("", false)]
fn the_configured_list_selects_the_check(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(settings(source).enforce_model_rate_limits, expected);
}
