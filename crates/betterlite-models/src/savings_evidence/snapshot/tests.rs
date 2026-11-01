#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    BaselineCostSnapshot, CostBreakdown, pricing_basis_from_breakdown, recorded_token_cost,
};

fn decimal(text: &str) -> Decimal {
    text.parse()
        .expect("the fixture is a valid decimal literal")
}

fn minimal_snapshot(extra: &str) -> String {
    let separator = if extra.is_empty() { "" } else { "," };
    format!(
        r#"{{"model": "claude-3-haiku", "provider": "anthropic", "prices": null, "actual_spend": 0.5{separator}{extra}}}"#
    )
}

fn snapshot(extra: &str) -> Result<BaselineCostSnapshot, serde_json::Error> {
    serde_json::from_str(&minimal_snapshot(extra))
}

fn cost_breakdown(text: &str) -> CostBreakdown {
    serde_json::from_str(text).expect("the fixture cost_breakdown deserializes")
}

#[test]
fn deserializes_required_fields_and_defaults_basis_and_classifier_cost() {
    let snapshot = snapshot("").expect("a minimal snapshot deserializes");
    assert_eq!(snapshot.actual_spend, decimal("0.5"));
    assert_eq!(snapshot.classifier_cost, Decimal::ZERO);
    assert_eq!(snapshot.actual_token_cost, None);
    assert_eq!(snapshot.basis.service_tier, None);
}

#[test]
fn rejects_an_unknown_field() {
    snapshot(r#""unexpected": 1"#).unwrap_err();
}

#[rstest]
#[case::bool_actual_spend(r#""actual_spend": true"#)]
#[case::string_actual_spend(r#""actual_spend": "0.5""#)]
#[case::negative_actual_spend(r#""actual_spend": -1.0"#)]
#[case::negative_actual_token_cost(r#""actual_token_cost": -1.0"#)]
#[case::bool_actual_token_cost(r#""actual_token_cost": true"#)]
#[case::negative_classifier_cost(r#""classifier_cost": -1.0"#)]
#[case::bool_classifier_cost(r#""classifier_cost": false"#)]
fn rejects_boolean_nonnumeric_and_negative_amounts(#[case] extra: &str) {
    snapshot(extra).unwrap_err();
}

#[test]
fn accepts_a_present_classifier_cost_and_token_cost() {
    let snapshot = snapshot(r#""actual_token_cost": 0.1, "classifier_cost": 0.02"#)
        .expect("valid amounts deserialize");
    assert_eq!(snapshot.actual_token_cost, Some(decimal("0.1")));
    assert_eq!(snapshot.classifier_cost, decimal("0.02"));
}

#[test]
fn deserializes_the_tuple_shaped_basis() {
    let snapshot = snapshot(r#""basis": ["premium", "eu", "us-central1"]"#)
        .expect("a tuple-shaped basis deserializes");
    assert_eq!(snapshot.basis.service_tier.as_deref(), Some("premium"));
    assert_eq!(snapshot.basis.data_residency.as_deref(), Some("eu"));
    assert_eq!(
        snapshot.basis.vertex_location.as_deref(),
        Some("us-central1")
    );
}

#[rstest]
#[case::missing(None, None, None, None)]
#[case::empty_mapping(Some("{}"), None, None, None)]
#[case::strings_pass_through(
    Some(
        r#"{"service_tier": "premium", "data_residency": "eu", "vertex_location": "us-central1"}"#
    ),
    Some("premium"),
    Some("eu"),
    Some("us-central1")
)]
#[case::nonstring_values_become_none(
    Some(r#"{"service_tier": 1, "data_residency": true, "vertex_location": null}"#),
    None,
    None,
    None
)]
fn pricing_basis_from_breakdown_extracts_only_strings(
    #[case] text: Option<&str>,
    #[case] service_tier: Option<&str>,
    #[case] data_residency: Option<&str>,
    #[case] vertex_location: Option<&str>,
) {
    let value = text.map(cost_breakdown);
    let basis = pricing_basis_from_breakdown(value.as_ref());
    assert_eq!(basis.service_tier.as_deref(), service_tier);
    assert_eq!(basis.data_residency.as_deref(), data_residency);
    assert_eq!(basis.vertex_location.as_deref(), vertex_location);
}

#[test]
fn recorded_token_cost_sums_input_and_output_only() {
    let value = cost_breakdown(
        r#"{"input_cost": 0.4, "output_cost": 0.1, "cache_read_cost": 10.0, "cache_creation_cost": 10.0}"#,
    );
    assert_eq!(recorded_token_cost(Some(&value)), Some(decimal("0.5")));
}

#[test]
fn recorded_token_cost_accepts_boolean_components() {
    let value = cost_breakdown(r#"{"input_cost": true, "output_cost": false}"#);
    assert_eq!(recorded_token_cost(Some(&value)), Some(Decimal::ONE));
}

#[rstest]
#[case::missing_cost_breakdown(None)]
#[case::empty_cost_breakdown(Some("{}"))]
#[case::string_output_cost(Some(r#"{"input_cost": 1.0, "output_cost": "1.0"}"#))]
#[case::missing_output_cost(Some(r#"{"input_cost": 1.0}"#))]
fn recorded_token_cost_is_none_when_either_component_is_not_numeric(#[case] text: Option<&str>) {
    let value = text.map(cost_breakdown);
    assert_eq!(recorded_token_cost(value.as_ref()), None);
}
