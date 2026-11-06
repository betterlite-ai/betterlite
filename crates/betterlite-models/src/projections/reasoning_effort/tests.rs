#![cfg(test)]

use rstest::rstest;

use super::{ReasoningEffort, effort_from_thinking_budget};

#[rstest]
#[case::minimal_for_a_negative_budget(-1, ReasoningEffort::Minimal)]
#[case::minimal_for_a_zero_budget(0, ReasoningEffort::Minimal)]
#[case::minimal_below_the_low_threshold(1023, ReasoningEffort::Minimal)]
#[case::low_at_the_low_threshold(1024, ReasoningEffort::Low)]
#[case::low_below_the_medium_threshold(2047, ReasoningEffort::Low)]
#[case::medium_at_the_medium_threshold(2048, ReasoningEffort::Medium)]
#[case::medium_below_the_high_threshold(4095, ReasoningEffort::Medium)]
#[case::high_at_the_high_threshold(4096, ReasoningEffort::High)]
#[case::high_above_the_high_threshold(8192, ReasoningEffort::High)]
#[case::high_for_the_largest_budget(i64::MAX, ReasoningEffort::High)]
fn parameter_projections_behavior_tests_behavior(
    #[case] budget_tokens: i64,
    #[case] expected: ReasoningEffort,
) {
    assert_eq!(effort_from_thinking_budget(budget_tokens), expected);
}
