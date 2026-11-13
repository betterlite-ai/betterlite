#![cfg(test)]
use rstest::rstest;

use super::budget_model_candidates;
use crate::model_budget_values::tests::support::catalog;

#[rstest]
#[case::bare_model("gpt-4", &["gpt-4"])]
#[case::provider_prefix("openai/gpt-4o", &["openai/gpt-4o", "gpt-4o"])]
#[case::nested_prefix("a/b/c", &["a/b/c", "c"])]
#[case::trailing_slash("openai/", &["openai/", ""])]
#[case::dotted_name_not_in_the_cost_map("azure/gpt-4.1", &["azure/gpt-4.1", "gpt-4.1"])]
#[case::bedrock_cross_region(
    "us.anthropic.claude-opus-4-8",
    &["us.anthropic.claude-opus-4-8", "anthropic.claude-opus-4-8", "claude-opus-4-8"]
)]
#[case::bedrock_with_provider_prefix(
    "bedrock/us.anthropic.claude-opus-4-8",
    &[
        "bedrock/us.anthropic.claude-opus-4-8",
        "us.anthropic.claude-opus-4-8",
        "anthropic.claude-opus-4-8",
        "claude-opus-4-8",
    ]
)]
#[case::bedrock_base_already_named(
    "anthropic.claude-opus-4-8",
    &["anthropic.claude-opus-4-8", "claude-opus-4-8"]
)]
#[case::bedrock_base_without_a_dot("titan", &["titan"])]
#[case::bedrock_base_ending_in_a_dot("us.amazon.", &["us.amazon.", "amazon."])]
fn candidates_run_from_the_most_specific_name(#[case] model: &str, #[case] expected: &[&str]) {
    assert_eq!(budget_model_candidates(model, &catalog()), expected);
}

#[test]
fn candidates_have_no_repeats_when_the_model_is_its_own_base() {
    let candidates = budget_model_candidates("titan", &catalog());
    assert_eq!(candidates.len(), 1);
}
