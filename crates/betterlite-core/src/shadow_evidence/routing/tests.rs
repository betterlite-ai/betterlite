#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{classifier_cost, decision, routed_tier};

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[test]
fn decision_reads_the_mapping_metadata_carries() {
    let metadata = json(r#"{"routing_decision":{"tier":"low","routed_model":"m"}}"#);
    let fields = decision(&metadata);
    assert_eq!(fields.get("tier"), Some(&json(r#""low""#)));
    assert_eq!(fields.get("routed_model"), Some(&json(r#""m""#)));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"routing_decision":null}"#)]
#[case::text(r#"{"routing_decision":"x"}"#)]
#[case::list(r#"{"routing_decision":[1]}"#)]
#[case::number(r#"{"routing_decision":5}"#)]
#[case::bool(r#"{"routing_decision":true}"#)]
#[case::metadata_not_an_object("5")]
fn decision_is_empty_without_a_mapping(#[case] metadata: &str) {
    assert!(decision(&json(metadata)).is_empty());
}

#[rstest]
#[case::label_wins(
    r#"{"routing_decision":{"tier_label":"high","tier":"low"}}"#,
    Some("high")
)]
#[case::tier_without_a_label(r#"{"routing_decision":{"tier":"low"}}"#, Some("low"))]
#[case::empty_label_falls_to_tier(
    r#"{"routing_decision":{"tier_label":"","tier":"low"}}"#,
    Some("low")
)]
#[case::null_label_falls_to_tier(
    r#"{"routing_decision":{"tier_label":null,"tier":"low"}}"#,
    Some("low")
)]
#[case::zero_label_falls_to_tier(
    r#"{"routing_decision":{"tier_label":0,"tier":"low"}}"#,
    Some("low")
)]
#[case::false_label_falls_to_tier(
    r#"{"routing_decision":{"tier_label":false,"tier":"low"}}"#,
    Some("low")
)]
#[case::empty_list_label_falls_to_tier(
    r#"{"routing_decision":{"tier_label":[],"tier":"low"}}"#,
    Some("low")
)]
#[case::empty_object_label_falls_to_tier(
    r#"{"routing_decision":{"tier_label":{},"tier":"low"}}"#,
    Some("low")
)]
#[case::empty_tier_is_kept(r#"{"routing_decision":{"tier_label":"","tier":""}}"#, Some(""))]
#[case::null_tier_is_none(r#"{"routing_decision":{"tier":null}}"#, None)]
#[case::no_tier_at_all(r#"{"routing_decision":{"routed_model":"m"}}"#, None)]
#[case::no_decision(r#"{"tier":"low"}"#, None)]
#[case::number_tier_is_stringified(r#"{"routing_decision":{"tier":5}}"#, Some("5"))]
#[case::false_tier_is_stringified(r#"{"routing_decision":{"tier":false}}"#, Some("False"))]
#[case::true_tier_is_stringified(r#"{"routing_decision":{"tier":true}}"#, Some("True"))]
#[case::list_tier_keeps_its_text(r#"{"routing_decision":{"tier":["low"]}}"#, Some(r#"["low"]"#))]
#[case::empty_object_tier_is_stringified(r#"{"routing_decision":{"tier":{}}}"#, Some("{}"))]
fn routed_tier_prefers_the_label(#[case] metadata: &str, #[case] expected: Option<&str>) {
    assert_eq!(routed_tier(&json(metadata)).as_deref(), expected);
}

#[rstest]
#[case::decimal(
    r#"{"routing_decision":{"classifier_cost":0.0012}}"#,
    Decimal::new(12, 4)
)]
#[case::integer(r#"{"routing_decision":{"classifier_cost":3}}"#, Decimal::new(3, 0))]
#[case::exponent(r#"{"routing_decision":{"classifier_cost":1e-3}}"#, Decimal::new(1, 3))]
#[case::absent(r#"{"routing_decision":{"tier":"low"}}"#, Decimal::ZERO)]
#[case::text(r#"{"routing_decision":{"classifier_cost":"0.5"}}"#, Decimal::ZERO)]
#[case::true_is_not_a_number(r#"{"routing_decision":{"classifier_cost":true}}"#, Decimal::ZERO)]
#[case::null(r#"{"routing_decision":{"classifier_cost":null}}"#, Decimal::ZERO)]
#[case::unrepresentable(r#"{"routing_decision":{"classifier_cost":1e-40}}"#, Decimal::ZERO)]
#[case::no_decision("{}", Decimal::ZERO)]
fn classifier_cost_reads_a_number(#[case] metadata: &str, #[case] expected: Decimal) {
    assert_eq!(classifier_cost(&json(metadata)), expected);
}
