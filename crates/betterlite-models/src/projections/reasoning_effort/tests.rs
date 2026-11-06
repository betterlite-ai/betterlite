#![cfg(test)]

use rstest::rstest;

use super::{ReasoningEffort, ThinkingBudgetThresholds, effort_from_thinking_budget};

const OVERRIDDEN: ThinkingBudgetThresholds = ThinkingBudgetThresholds {
    low: 512,
    medium: 1024,
    high: 2048,
};

#[rstest]
#[case::minimal_for_a_negative_budget(-1, ThinkingBudgetThresholds::default(), ReasoningEffort::Minimal)]
#[case::minimal_for_a_zero_budget(0, ThinkingBudgetThresholds::default(), ReasoningEffort::Minimal)]
#[case::minimal_below_the_low_threshold(
    1023,
    ThinkingBudgetThresholds::default(),
    ReasoningEffort::Minimal
)]
#[case::low_at_the_low_threshold(1024, ThinkingBudgetThresholds::default(), ReasoningEffort::Low)]
#[case::low_below_the_medium_threshold(
    2047,
    ThinkingBudgetThresholds::default(),
    ReasoningEffort::Low
)]
#[case::medium_at_the_medium_threshold(
    2048,
    ThinkingBudgetThresholds::default(),
    ReasoningEffort::Medium
)]
#[case::medium_below_the_high_threshold(
    4095,
    ThinkingBudgetThresholds::default(),
    ReasoningEffort::Medium
)]
#[case::high_at_the_high_threshold(
    4096,
    ThinkingBudgetThresholds::default(),
    ReasoningEffort::High
)]
#[case::high_above_the_high_threshold(
    8192,
    ThinkingBudgetThresholds::default(),
    ReasoningEffort::High
)]
#[case::high_for_the_largest_budget(
    i64::MAX,
    ThinkingBudgetThresholds::default(),
    ReasoningEffort::High
)]
#[case::override_minimal_below_the_low_threshold(511, OVERRIDDEN, ReasoningEffort::Minimal)]
#[case::override_low_at_the_low_threshold(512, OVERRIDDEN, ReasoningEffort::Low)]
#[case::override_low_below_the_medium_threshold(1023, OVERRIDDEN, ReasoningEffort::Low)]
#[case::override_medium_at_the_medium_threshold(1024, OVERRIDDEN, ReasoningEffort::Medium)]
#[case::override_medium_below_the_high_threshold(2047, OVERRIDDEN, ReasoningEffort::Medium)]
#[case::override_high_at_the_high_threshold(2048, OVERRIDDEN, ReasoningEffort::High)]
fn parameter_projections_behavior_tests_behavior(
    #[case] budget_tokens: i64,
    #[case] thresholds: ThinkingBudgetThresholds,
    #[case] expected: ReasoningEffort,
) {
    assert_eq!(
        effort_from_thinking_budget(budget_tokens, thresholds),
        expected
    );
}
