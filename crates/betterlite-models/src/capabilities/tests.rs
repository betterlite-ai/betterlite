#![cfg(test)]

use super::CapabilityFields;
use rstest::rstest;

#[rstest]
#[case::absent("{}", None)]
#[case::explicit_false(r#"{"supports_vision":false}"#, Some(false))]
#[case::explicit_true(r#"{"supports_vision":true}"#, Some(true))]
fn model_metadata_types_model_field_tests_behavior(
    #[case] source: &str,
    #[case] expected: Option<bool>,
) {
    let fields: CapabilityFields = serde_json::from_str(source).unwrap();
    assert_eq!(fields.media.supports_vision, expected);
}

#[test]
fn absent_flags_round_trip_without_the_key() {
    let fields: CapabilityFields = serde_json::from_str("{}").unwrap();
    let projected = serde_json::to_string(&fields).unwrap();
    assert_eq!(projected, "{}");
}

#[test]
fn an_unrecognized_key_is_ignored_rather_than_rejected() {
    let fields: CapabilityFields =
        serde_json::from_str(r#"{"supports_vision":true,"not_a_flag":true}"#).unwrap();
    assert_eq!(fields.media.supports_vision, Some(true));
}
