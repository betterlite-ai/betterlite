#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{CallFailure, JudgeVerdict, ShadowResponse};

#[test]
fn call_failure_keeps_the_error_and_zero_costs() {
    let failure = CallFailure {
        error: "empty response".to_owned(),
        cost: Decimal::ZERO,
        classifier_cost: Decimal::ZERO,
    };
    assert_eq!(failure.error, "empty response");
    assert_eq!(failure.cost, Decimal::ZERO);
    assert_eq!(failure.classifier_cost, Decimal::ZERO);
}

#[test]
fn shadow_response_keeps_the_tier_when_there_is_one() {
    let response = ShadowResponse {
        text: "hello".to_owned(),
        model: "gpt-4o".to_owned(),
        tier: Some("high".to_owned()),
        cost: Decimal::new(123, 5),
        classifier_cost: Decimal::new(12, 4),
    };
    assert_eq!(response.tier.as_deref(), Some("high"));
    assert_eq!(response.cost, Decimal::new(123, 5));
    assert_eq!(response.classifier_cost, Decimal::new(12, 4));
}

#[test]
fn shadow_response_keeps_a_missing_tier() {
    let response = ShadowResponse {
        text: String::new(),
        model: "gpt-4o".to_owned(),
        tier: None,
        cost: Decimal::ZERO,
        classifier_cost: Decimal::ZERO,
    };
    assert_eq!(response.tier, None);
    assert_eq!(response.text, "");
}

#[rstest]
#[case::within_the_range(Decimal::new(9, 1), Decimal::new(9, 1))]
#[case::zero_stays(Decimal::ZERO, Decimal::ZERO)]
#[case::one_stays(Decimal::ONE, Decimal::ONE)]
#[case::above_one(Decimal::new(15, 1), Decimal::ONE)]
#[case::below_zero(Decimal::new(-2, 1), Decimal::ZERO)]
#[case::large_above(Decimal::new(900, 0), Decimal::ONE)]
#[case::large_below(Decimal::new(-900, 0), Decimal::ZERO)]
fn judge_verdict_clamps_confidence(#[case] confidence: Decimal, #[case] expected: Decimal) {
    let verdict = JudgeVerdict::parsed("A".to_owned(), confidence, Decimal::new(25, 4));
    assert_eq!(verdict.preference, "A");
    assert_eq!(verdict.confidence, expected);
    assert_eq!(verdict.cost, Decimal::new(25, 4));
}
