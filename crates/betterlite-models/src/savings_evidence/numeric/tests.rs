#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{RoutingDecision, numeric_savings, recorded_autorouter_savings};
use crate::savings_evidence::metadata::Metadata;
use crate::tiered_pricing::Scalar;

fn scalar(text: &str) -> Scalar {
    serde_json::from_str(text).expect("the fixture is valid scalar JSON")
}

fn decimal(text: &str) -> Decimal {
    text.parse()
        .expect("the fixture is a valid decimal literal")
}

#[rstest]
#[case::integer("5", Some("5"))]
#[case::fraction("0.5", Some("0.5"))]
#[case::negative("-2.5", Some("-2.5"))]
#[case::boolean("true", None)]
#[case::string("\"5\"", None)]
#[case::null("null", None)]
#[case::array("[1]", None)]
fn numeric_savings_accepts_finite_signed_numbers_only(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let value = scalar(text);
    assert_eq!(numeric_savings(Some(&value)), expected.map(decimal));
}

#[test]
fn numeric_savings_treats_a_missing_value_as_none() {
    assert_eq!(numeric_savings(None), None);
}

fn metadata_with_estimate(version_text: &str, status: &str) -> Metadata {
    let estimate = format!(r#"{{"version": {version_text}, "status": "{status}"}}"#);
    serde_json::from_str(&format!(
        r#"{{"autorouter_savings_estimate": {estimate}, "autorouter_savings": 3}}"#
    ))
    .expect("the fixture metadata deserializes")
}

#[rstest]
#[case::version_one("1", "estimated", Some("3"))]
#[case::version_three("3", "estimated", Some("3"))]
#[case::version_bool("true", "estimated", None)]
#[case::version_float("2.0", "estimated", None)]
#[case::version_out_of_range("4", "estimated", None)]
#[case::status_mismatch("1", "pending", None)]
fn recorded_autorouter_savings_requires_exact_version_and_status(
    #[case] version_text: &str,
    #[case] status: &str,
    #[case] expected: Option<&str>,
) {
    let metadata = metadata_with_estimate(version_text, status);
    assert_eq!(
        recorded_autorouter_savings(&metadata),
        expected.map(decimal)
    );
}

#[test]
fn recorded_autorouter_savings_is_none_without_an_estimate() {
    assert_eq!(recorded_autorouter_savings(&Metadata::default()), None);
}

#[test]
fn numeric_savings_reads_a_routing_decisions_classifier_cost() {
    let decision: RoutingDecision = serde_json::from_str(r#"{"classifier_cost": 0.002}"#)
        .expect("the fixture decision deserializes");
    assert_eq!(
        numeric_savings(Some(&decision).and_then(|value| value.classifier_cost.as_ref())),
        Some(decimal("0.002"))
    );
}

#[test]
fn numeric_savings_treats_a_missing_routing_decision_as_empty() {
    let routing_decision: Option<&RoutingDecision> = None;
    assert_eq!(
        numeric_savings(routing_decision.and_then(|value| value.classifier_cost.as_ref())),
        None
    );
}
