#![cfg(test)]

use rstest::rstest;
use serde_json::json;

use crate::estimator::extraction::{
    ExtractionError, MAX_REPLY_BYTES, first_json_object, mask_value_constants,
};
use crate::estimator::result::{
    ROIEstimatorResult, ResultError, validate_hours, validate_reasoning,
};

fn parse_result(raw: &str) -> Result<ROIEstimatorResult, serde_json::Error> {
    serde_json::from_str(&mask_value_constants(raw))
}

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
#[case::unit_separator("\u{1f}")]
#[case::information_separators("\u{1c}\u{1d}\u{1e}")]
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
fn parse_rejects_a_non_object_and_names_the_expected_shape() {
    let error = parse_result("[1, 2]").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("an object with hours and reasoning")
    );
}

#[rstest]
#[case::hours(r#"{"hours": 1, "hours": 2, "reasoning": "ok"}"#, 2.0, "ok")]
#[case::reasoning(
    r#"{"hours": 1, "reasoning": "first", "reasoning": "second"}"#,
    1.0,
    "second"
)]
fn parse_keeps_the_last_value_of_a_repeated_key(
    #[case] raw: &str,
    #[case] hours: f64,
    #[case] reasoning: &str,
) {
    let result = parse_result(raw).unwrap();
    assert_eq!(result.hours, hours);
    assert_eq!(result.reasoning, reasoning);
}

#[test]
fn parse_ignores_an_earlier_value_of_the_wrong_type() {
    let result =
        parse_result(r#"{"hours": "1", "hours": 2, "reasoning": 5, "reasoning": "ok"}"#).unwrap();
    assert_eq!(result.hours, 2.0);
    assert_eq!(result.reasoning, "ok");
}

#[test]
fn parse_rejects_a_last_value_of_the_wrong_type() {
    let error = parse_result(r#"{"hours": 2, "hours": "1", "reasoning": "ok"}"#).unwrap_err();
    assert!(error.to_string().contains("invalid type"));
}

#[test]
fn parse_rejects_an_unknown_key_beside_a_repeated_one() {
    let error =
        parse_result(r#"{"hours": 1, "hours": 2, "reasoning": "ok", "extra": true}"#).unwrap_err();
    assert!(error.to_string().contains("unknown field `extra`"));
}

#[rstest]
#[case::nan_hours(r#"{"hours": NaN, "hours": 2, "reasoning": "x"}"#)]
#[case::negative_infinity_hours(r#"{"hours": -Infinity, "hours": 2, "reasoning": "x"}"#)]
#[case::infinity_reasoning(r#"{"hours": 2, "reasoning": Infinity, "reasoning": "x"}"#)]
#[case::nan_inside_an_array(r#"{"hours": [NaN, 1], "hours": 2, "reasoning": "x"}"#)]
fn parse_keeps_the_last_value_past_a_python_constant(#[case] raw: &str) {
    let result = parse_result(raw).unwrap();
    assert_eq!(result.hours, 2.0);
    assert_eq!(result.reasoning, "x");
}

#[rstest]
#[case::nan(r#"{"hours": 2, "hours": NaN, "reasoning": "x"}"#)]
#[case::infinity(r#"{"hours": 2, "hours": Infinity, "reasoning": "x"}"#)]
#[case::negative_infinity(r#"{"hours": 2, "hours": -Infinity, "reasoning": "x"}"#)]
fn parse_rejects_a_python_constant_as_the_last_hours(#[case] raw: &str) {
    let error = parse_result(raw).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Hours must be finite and nonnegative.")
    );
}

#[test]
fn parse_keeps_python_constant_words_inside_reasoning_text() {
    let result =
        parse_result(r#"{"hours": 2, "reasoning": "works NaN and Infinity fine"}"#).unwrap();
    assert_eq!(result.reasoning, "works NaN and Infinity fine");
}

#[rstest]
#[case::nan(r#"{"hours": NaN}"#, r#"{"hours": -1 }"#)]
#[case::infinity(r#"{"hours": Infinity}"#, r#"{"hours": -1000000}"#)]
#[case::negative_infinity(r#"{"hours": -Infinity}"#, r#"{"hours": -10000000}"#)]
#[case::inside_a_string(r#"{"reasoning": "NaN"}"#, r#"{"reasoning": "NaN"}"#)]
#[case::glued_to_a_digit(r#"{"hours": 1NaN}"#, r#"{"hours": 1NaN}"#)]
#[case::glued_to_a_letter(r#"{"hours": NaN5}"#, r#"{"hours": NaN5}"#)]
#[case::glued_at_both_ends(r#"{"hours": 1NaN5}"#, r#"{"hours": 1NaN5}"#)]
fn mask_value_constants_rewrites_only_python_constants_in_value_positions(
    #[case] raw: &str,
    #[case] masked: &str,
) {
    assert_eq!(mask_value_constants(raw), masked);
}

#[test]
fn parse_extracts_the_first_complete_object() {
    let object = first_json_object(
        "Sure:\n```json\n{\"hours\": 1, \"reasoning\": \"first\"}\n```\n{\"hours\": 2, \"reasoning\": \"second\"}",
    )
    .unwrap();
    let result = parse_result(object).unwrap();
    assert_eq!(result.hours, 1.0);
    assert_eq!(result.reasoning, "first");
}

#[test]
fn parse_strips_prose_without_an_object() {
    let content = first_json_object("  no json here  ").unwrap();
    assert_eq!(content, "no json here");
    assert!(parse_result(content).is_err());
}

#[rstest]
#[case::nan(r#"{"hours": NaN, "reasoning": "x"}"#)]
#[case::infinity(r#"{"hours": Infinity, "reasoning": "x"}"#)]
#[case::negative_infinity(r#"{"hours": -Infinity, "reasoning": "x"}"#)]
#[case::nested_in_array(r#"{"hours": [NaN, 1], "reasoning": "x"}"#)]
fn extraction_picks_an_object_holding_a_python_constant_over_a_later_valid_one(
    #[case] first: &str,
) {
    let content = format!(r#"{first} {{"hours": 1, "reasoning": "y"}}"#);
    let object = first_json_object(&content).unwrap();
    assert_eq!(object, first);
    assert!(parse_result(object).is_err());
}

#[rstest]
#[case::digit_before(r#"{"hours": 1NaN, "reasoning": "x"}"#)]
#[case::letter_after(r#"{"hours": NaNa, "reasoning": "x"}"#)]
#[case::digit_after(r#"{"hours": NaN5, "reasoning": "x"}"#)]
#[case::sign_before_nan(r#"{"hours": -NaN, "reasoning": "x"}"#)]
fn extraction_skips_an_object_whose_constant_is_not_a_json_token(#[case] broken: &str) {
    let valid = r#"{"hours": 1, "reasoning": "y"}"#;
    let content = format!("{broken} {valid}");
    assert_eq!(first_json_object(&content).unwrap(), valid);
}

#[test]
fn extraction_accepts_a_reply_of_exactly_the_byte_limit() {
    let object = r#"{"hours": 1, "reasoning": "y"}"#;
    let content = format!("{object}{}", " ".repeat(MAX_REPLY_BYTES - object.len()));
    assert_eq!(content.len(), MAX_REPLY_BYTES);
    assert_eq!(first_json_object(&content).unwrap(), object);
}

#[test]
fn extraction_refuses_a_reply_one_byte_over_the_limit() {
    let content = " ".repeat(MAX_REPLY_BYTES + 1);
    assert_eq!(
        first_json_object(&content),
        Err(ExtractionError::TooLarge {
            bytes: MAX_REPLY_BYTES + 1,
            limit: MAX_REPLY_BYTES,
        })
    );
}

#[test]
fn extraction_gives_up_when_failed_starts_use_the_scan_budget() {
    let content = format!("{}x", "{\"a\":".repeat(200_000));
    assert_eq!(
        first_json_object(&content),
        Err(ExtractionError::ScanBudget)
    );
}

#[test]
fn extraction_charges_quick_failures_only_the_bytes_they_read() {
    let object = r#"{"hours": 1, "reasoning": "y"}"#;
    let content = format!("{}{object}", "{x".repeat(200_000));
    assert_eq!(first_json_object(&content).unwrap(), object);
}

#[test]
fn extraction_finds_an_object_after_multibyte_text() {
    let object = r#"{"hours": 1, "reasoning": "é"}"#;
    let content = format!("héllo wörld {object}");
    assert_eq!(first_json_object(&content).unwrap(), object);
}

#[test]
fn validate_reasoning_matches_pythons_truthiness_for_blank_text() {
    assert_eq!(validate_reasoning("ok".to_owned()), Ok("ok".to_owned()));
    assert_eq!(
        validate_reasoning(" \u{a0} ".to_owned()),
        Err(ResultError::Reasoning)
    );
    assert_eq!(
        validate_reasoning("\u{1f}kept ".to_owned()),
        Ok("\u{1f}kept ".to_owned())
    );
}
