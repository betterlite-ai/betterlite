use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::vigil_guard::metadata::{
    MetadataScalar, MetadataValue, clamp_metadata_value,
};

use crate::support::text;

#[test]
fn keeps_strings_within_the_limit() {
    assert_eq!(
        clamp_metadata_value(&text("inside")),
        Some(MetadataValue::Text("inside".to_owned()))
    );
    assert_eq!(
        clamp_metadata_value(&text(&"x".repeat(500))),
        Some(MetadataValue::Text("x".repeat(500)))
    );
}

#[test]
fn truncates_strings_to_500_characters() {
    assert_eq!(
        clamp_metadata_value(&text(&"x".repeat(501))),
        Some(MetadataValue::Text("x".repeat(500)))
    );
}

#[test]
fn truncates_by_characters_not_bytes() {
    let clamped = clamp_metadata_value(&text(&"\u{e9}".repeat(600)));
    assert_eq!(clamped, Some(MetadataValue::Text("\u{e9}".repeat(500))));
}

#[test]
fn truncates_arrays_to_ten_items() {
    let items: Vec<GuardrailValue> = (0_i32..12_i32)
        .map(|index| GuardrailValue::SignedInteger(i64::from(index)))
        .collect();
    let clamped = clamp_metadata_value(&GuardrailValue::Array(items));
    let expected: Vec<MetadataScalar> = (0_i32..10_i32)
        .map(|index| MetadataScalar::SignedInteger(i64::from(index)))
        .collect();
    assert_eq!(clamped, Some(MetadataValue::List(expected)));
}

#[test]
fn drops_booleans_from_arrays() {
    let items = vec![
        GuardrailValue::Boolean(true),
        text("kept"),
        GuardrailValue::Boolean(false),
    ];
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::Array(items)),
        Some(MetadataValue::List(vec![MetadataScalar::Text(
            "kept".to_owned()
        )]))
    );
}

#[test]
fn drops_non_scalar_array_items() {
    let items = vec![
        GuardrailValue::Null,
        GuardrailValue::Array(Vec::new()),
        GuardrailValue::Object(BTreeMap::new()),
        text("kept"),
    ];
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::Array(items)),
        Some(MetadataValue::List(vec![MetadataScalar::Text(
            "kept".to_owned()
        )]))
    );
}

#[test]
fn an_empty_array_is_none() {
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::Array(Vec::new())),
        None
    );
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::Array(vec![GuardrailValue::Boolean(true)])),
        None
    );
}

#[test]
fn keeps_numbers_as_given() {
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::SignedInteger(-3)),
        Some(MetadataValue::SignedInteger(-3))
    );
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::UnsignedInteger(7)),
        Some(MetadataValue::UnsignedInteger(7))
    );
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::Decimal(0.5)),
        Some(MetadataValue::Decimal(0.5))
    );
}

#[test]
fn null_boolean_and_object_values_are_none() {
    assert_eq!(clamp_metadata_value(&GuardrailValue::Null), None);
    assert_eq!(clamp_metadata_value(&GuardrailValue::Boolean(true)), None);
    assert_eq!(
        clamp_metadata_value(&GuardrailValue::Object(BTreeMap::new())),
        None
    );
}
