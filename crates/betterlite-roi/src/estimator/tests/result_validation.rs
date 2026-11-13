#![cfg(test)]

use rstest::rstest;
use serde_json::json;

use crate::estimator::result::{
    ROIEstimatorResult, ResultError, parse_estimator_result, validate_hours, validate_reasoning,
};

#[rstest]
#[case::integer(json!({"hours": 3, "reasoning": "ok"}), 3.0)]
#[case::float(json!({"hours": 2.5, "reasoning": "ok"}), 2.5)]
#[case::zero(json!({"hours": 0, "reasoning": "ok"}), 0.0)]
#[case::large(json!({"hours": 12000.75, "reasoning": "ok"}), 12000.75)]
fn parse_accepts_finite_nonnegative_hours(#[case] value: serde_json::Value, #[case] hours: f64) {
    let result: ROIEstimatorResult = serde_json::from_value(value).unwrap();
    assert_eq!(result.hours, hours);
}

#[rstest]
#[case::negative(json!({"hours": -1, "reasoning": "ok"}))]
#[case::negative_float(json!({"hours": -0.5, "reasoning": "ok"}))]
fn parse_rejects_negative_hours(#[case] value: serde_json::Value) {
    let error = serde_json::from_value::<ROIEstimatorResult>(value).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Hours must be finite and nonnegative.")
    );
}

#[test]
fn validate_hours_rejects_nan_and_infinities() {
    assert_eq!(validate_hours(f64::NAN), Err(ResultError::Hours));
    assert_eq!(validate_hours(f64::INFINITY), Err(ResultError::Hours));
    assert_eq!(validate_hours(f64::NEG_INFINITY), Err(ResultError::Hours));
}

#[test]
fn validate_hours_accepts_zero_and_positive_values() {
    assert_eq!(validate_hours(0.0), Ok(0.0));
    assert_eq!(validate_hours(0.25), Ok(0.25));
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::newlines("\n\t ")]
fn parse_rejects_blank_reasoning(#[case] reasoning: &str) {
    let value = json!({"hours": 1, "reasoning": reasoning});
    let error = serde_json::from_value::<ROIEstimatorResult>(value).unwrap_err();
    assert!(error.to_string().contains("Reasoning must not be empty."));
}

#[test]
fn parse_keeps_reasoning_whitespace_around_text() {
    let result: ROIEstimatorResult =
        serde_json::from_value(json!({"hours": 1, "reasoning": "  kept  "})).unwrap();
    assert_eq!(result.reasoning, "  kept  ");
}

#[rstest]
#[case::extra_field(json!({"hours": 1, "reasoning": "ok", "extra": 2}))]
#[case::string_hours(json!({"hours": "1", "reasoning": "ok"}))]
#[case::bool_hours(json!({"hours": true, "reasoning": "ok"}))]
#[case::missing_hours(json!({"reasoning": "ok"}))]
#[case::missing_reasoning(json!({"hours": 1}))]
fn parse_is_strict(#[case] value: serde_json::Value) {
    assert!(serde_json::from_value::<ROIEstimatorResult>(value).is_err());
}

#[test]
fn parse_extracts_the_first_complete_object() {
    let result = parse_estimator_result(
        "Sure:\n```json\n{\"hours\": 1, \"reasoning\": \"first\"}\n```\n{\"hours\": 2, \"reasoning\": \"second\"}",
    )
    .unwrap();
    assert_eq!(result.hours, 1.0);
    assert_eq!(result.reasoning, "first");
}

#[test]
fn parse_strips_prose_without_an_object() {
    assert!(parse_estimator_result("  no json here  ").is_err());
}

#[test]
fn validate_reasoning_matches_pythons_truthiness_for_blank_text() {
    assert_eq!(validate_reasoning("ok".to_owned()), Ok("ok".to_owned()));
    assert_eq!(
        validate_reasoning(" \u{a0} ".to_owned()),
        Err(ResultError::Reasoning)
    );
}
