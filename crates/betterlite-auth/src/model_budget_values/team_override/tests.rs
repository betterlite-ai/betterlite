#![cfg(test)]
use rstest::rstest;

use super::team_model_budget_applies;
use crate::model_budget_values::tests::support::{budgets, catalog, runtime};

#[rstest]
#[case::spend_gated(
    "gpt-4o",
    r#"{"gpt-4o": {"max_budget": 5, "budget_duration": "1d"}}"#,
    false
)]
#[case::zero_budget_still_gates(
    "gpt-4o",
    r#"{"gpt-4o": {"max_budget": 0, "budget_duration": "1d"}}"#,
    false
)]
#[case::gated_through_the_bare_name(
    "openai/gpt-4o",
    r#"{"gpt-4o": {"max_budget": 5, "budget_duration": "1d"}}"#,
    false
)]
#[case::gated_through_the_reported_name(
    "gpt-4o",
    r#"{"gpt-4o": {"budget_limit": 5, "time_period": "1d"}}"#,
    false
)]
#[case::rate_limits_only(
    "gpt-4o",
    r#"{"gpt-4o": {"budget_duration": "1d", "tpm_limit": 5}}"#,
    true
)]
#[case::negative_budget_does_not_gate(
    "gpt-4o",
    r#"{"gpt-4o": {"max_budget": -1, "budget_duration": "1d"}}"#,
    true
)]
#[case::other_model_configured(
    "gpt-4o",
    r#"{"claude": {"max_budget": 5, "budget_duration": "1d"}}"#,
    true
)]
#[case::malformed_entry_does_not_gate("gpt-4o", r#"{"gpt-4o": {"max_budget": 5}}"#, true)]
#[case::no_entries("gpt-4o", "{}", true)]
fn team_override_follows_the_key_budget_for_the_model(
    #[case] model: &str,
    #[case] key_budgets: &str,
    #[case] expected: bool,
) {
    let parsed = budgets(key_budgets);
    assert_eq!(
        team_model_budget_applies(model, Some(&parsed), &catalog(), runtime()),
        expected
    );
}

#[test]
fn team_override_applies_when_the_key_has_no_model_budgets() {
    assert!(team_model_budget_applies(
        "gpt-4o",
        None,
        &catalog(),
        runtime()
    ));
}
