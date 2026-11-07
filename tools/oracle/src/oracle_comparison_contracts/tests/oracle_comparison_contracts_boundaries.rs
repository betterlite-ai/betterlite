use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::oracle_comparison_contracts::{
    ContractError, ExpectedDifference, NumericRule, compare_documents, validate,
};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn numeric(pointer: &str, tolerance: Decimal) -> NumericRule {
    NumericRule {
        pointer: pointer.to_owned(),
        absolute_tolerance: tolerance,
        why: "the pointer carries a float diagnostic".to_owned(),
    }
}

fn expected(pointer: &str, reference: Value, rust: Value) -> ExpectedDifference {
    ExpectedDifference {
        pointer: pointer.to_owned(),
        reference,
        rust,
        why: "the declared difference pins both sides".to_owned(),
        link: String::new(),
    }
}

#[test]
fn two_numeric_rules_cannot_name_the_same_pointer() {
    let rules = [
        numeric("/score", Decimal::ONE),
        numeric("/score", Decimal::ONE),
    ];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::Duplicate { pointer, .. }) if pointer == "/score"
    ));
}

#[test]
fn a_numeric_rule_and_an_expected_difference_cannot_share_a_pointer() {
    let numeric_rules = [numeric("/detail", Decimal::ONE)];
    let differences = [expected("/detail", value("1"), value("2"))];
    assert!(matches!(
        validate(&numeric_rules, &differences),
        Err(ContractError::Duplicate { pointer, .. }) if pointer == "/detail"
    ));
}

#[rstest]
#[case::numeric_names_the_parent("/score", "/score/detail")]
#[case::expected_names_the_parent("/score/detail", "/score")]
fn overlapping_pointers_are_rejected(
    #[case] numeric_pointer: &str,
    #[case] expected_pointer: &str,
) {
    let numeric_rules = [numeric(numeric_pointer, Decimal::ONE)];
    let differences = [expected(expected_pointer, value("1"), value("2"))];
    assert!(matches!(
        validate(&numeric_rules, &differences),
        Err(ContractError::Overlap { .. })
    ));
}

#[rstest]
#[case::no_leading_slash("score")]
#[case::bad_escape("/score~2")]
#[case::trailing_tilde("/score~")]
fn a_pointer_that_is_not_a_json_pointer_is_rejected(#[case] pointer: &str) {
    let rules = [numeric(pointer, Decimal::ONE)];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::Pointer { .. })
    ));
}

#[test]
fn a_negative_tolerance_is_rejected() {
    let rules = [numeric("/score", Decimal::NEGATIVE_ONE)];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::NegativeTolerance { pointer }) if pointer == "/score"
    ));
}

#[rstest]
#[case::numeric(true)]
#[case::expected_difference(false)]
fn a_rule_needs_a_reason(#[case] numeric_rule: bool) {
    let numeric_rules = [numeric("/score", Decimal::ONE)];
    let differences = [expected("/detail", value("1"), value("2"))];
    let outcome = if numeric_rule {
        let mut blank = numeric_rules[0].clone();
        blank.why = "  ".to_owned();
        validate(&[blank], &[])
    } else {
        let mut blank = differences[0].clone();
        blank.why = String::new();
        validate(&[], &[blank])
    };
    assert!(matches!(outcome, Err(ContractError::MissingText { .. })));
}

#[test]
fn a_link_that_is_not_https_is_rejected() {
    let mut rule = expected("/detail", value("1"), value("2"));
    rule.link = "http://example.invalid/litellm".to_owned();
    assert!(matches!(
        validate(&[], &[rule]),
        Err(ContractError::Link { .. })
    ));
}

#[rstest]
#[case::selected("/output/selected")]
#[case::tier("/output/tier")]
#[case::status_code("/billing/status_code")]
#[case::count("/values/retry_count")]
#[case::tokens("/usage/total_tokens")]
#[case::spend("/billing/spend")]
#[case::provider_requests("/upstream/provider_requests")]
#[case::permission("/auth/allowed")]
fn a_tolerance_on_a_decision_pointer_is_rejected(#[case] pointer: &str) {
    let rules = [numeric(pointer, Decimal::new(1, 12))];
    assert!(matches!(
        validate(&rules, &[]),
        Err(ContractError::ExactOnly { .. })
    ));
}

#[test]
fn a_score_pointer_may_carry_a_tolerance() {
    let rules = [numeric("/output/score", Decimal::new(1, 12))];
    validate(&rules, &[]).unwrap();
}

#[test]
fn an_expected_difference_naming_a_missing_litellm_path_is_a_mismatch() {
    let reference = value(r#"{"other": 1}"#);
    let rust = value(r#"{"detail": 2}"#);
    let differences = [expected("/detail", value("2"), value("2"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
    assert!(error.message.contains("missing"));
}

#[test]
fn a_missing_betterlite_path_is_a_mismatch() {
    let reference = value(r#"{"detail": 2}"#);
    let rust = value(r#"{"other": 2}"#);
    let differences = [expected("/detail", value("2"), value("2"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
}

#[test]
fn a_numeric_rule_on_a_non_number_is_a_mismatch() {
    let reference = value(r#"{"score": "high"}"#);
    let rust = value(r#"{"score": "high"}"#);
    let rules = [numeric("/score", Decimal::ONE)];
    let error = compare_documents(&reference, &rust, &rules, &[], false).unwrap_err();
    assert_eq!(error.pointer, "/score");
}

#[test]
fn a_declared_type_that_differs_from_the_value_is_a_mismatch() {
    let reference = value(r#"{"detail": "text"}"#);
    let rust = value(r#"{"detail": "text"}"#);
    let differences = [expected("/detail", value("1"), value("1"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
    assert!(error.message.contains("type"));
}

#[rstest]
#[case::at_the_boundary("1.000000000001", true)]
#[case::past_the_boundary("1.000000000002", false)]
fn the_absolute_tolerance_includes_its_boundary(#[case] rust_score: &str, #[case] passes: bool) {
    let reference = value(r#"{"score": 1.0}"#);
    let rust = value(&format!(r#"{{"score": {rust_score}}}"#));
    let rules = [numeric("/score", Decimal::new(1, 12))];
    let outcome = compare_documents(&reference, &rust, &rules, &[], false);
    assert_eq!(outcome.is_ok(), passes);
}

#[test]
fn a_betterlite_value_that_differs_from_the_declared_rust_value_is_a_mismatch() {
    let reference = value(r#"{"detail": 2}"#);
    let rust = value(r#"{"detail": 3}"#);
    let differences = [expected("/detail", value("2"), value("2"))];
    let error = compare_documents(&reference, &rust, &[], &differences, false).unwrap_err();
    assert_eq!(error.pointer, "/detail");
    assert!(error.message.contains("betterlite"));
}
