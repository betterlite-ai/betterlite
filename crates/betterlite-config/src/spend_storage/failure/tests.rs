#![cfg(test)]

use rstest::rstest;

use super::loads_spend_failure_settings;
use crate::spend_storage::SpendUpdates;

#[rstest]
#[case::both_exact_true(
    "general_settings:\n  disable_error_logs: true\n  disable_spend_updates: true\n",
    true,
    SpendUpdates::Disabled
)]
#[case::absent_keeps_the_defaults("general_settings: {}\n", false, SpendUpdates::Allowed)]
#[case::missing_section("litellm_settings:\n  max_budget: 5\n", false, SpendUpdates::Allowed)]
#[case::explicit_false(
    "general_settings:\n  disable_error_logs: false\n  disable_spend_updates: false\n",
    false,
    SpendUpdates::Allowed
)]
#[case::text_true_is_not_a_boolean(
    "general_settings:\n  disable_error_logs: \"true\"\n  disable_spend_updates: \"true\"\n",
    false,
    SpendUpdates::Allowed
)]
#[case::numbers_are_not_booleans(
    "general_settings:\n  disable_error_logs: 1\n  disable_spend_updates: 1\n",
    false,
    SpendUpdates::Allowed
)]
#[case::nulls_keep_the_defaults(
    "general_settings:\n  disable_error_logs: null\n  disable_spend_updates: null\n",
    false,
    SpendUpdates::Allowed
)]
#[case::either_flag_alone(
    "general_settings:\n  disable_error_logs: true\n",
    true,
    SpendUpdates::Allowed
)]
#[case::unrelated_keys_are_ignored(
    "general_settings:\n  disable_spend_updates: true\n  store_prompts_in_spend_logs: true\nlitellm_settings:\n  max_budget: 7\n",
    false,
    SpendUpdates::Disabled
)]
fn loads_spend_failure_settings(
    #[case] source: &str,
    #[case] disable_error_logs: bool,
    #[case] spend_updates: SpendUpdates,
) {
    let settings = loads_spend_failure_settings(source).expect("the failure settings load");
    assert_eq!(settings.disable_error_logs, disable_error_logs);
    assert_eq!(settings.spend_updates, spend_updates);
}

#[test]
fn rejects_a_yaml_document_that_does_not_parse() {
    let error = loads_spend_failure_settings("general_settings: [unclosed\n")
        .expect_err("the document is malformed");
    assert!(!error.to_string().is_empty());
}
