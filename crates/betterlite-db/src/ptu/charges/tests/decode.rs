use std::borrow::Cow;

use betterlite_models::tiered_pricing::Scalar;
use rstest::rstest;

use super::support::data;
use crate::budget_permission_rows::JsonData;
use crate::ptu::charges::decode::{decode_ptu_model_info, model_info_scalars};

#[test]
fn object_mapping_decodes_without_a_copy() {
    let raw = data(r#"{"ptu_count": 2, "team_id": "team-1"}"#);
    let decoded = decode_ptu_model_info(&raw).unwrap();
    assert!(matches!(decoded, Cow::Borrowed(_)));
    assert_eq!(
        decoded.get("team_id"),
        Some(&JsonData::String("team-1".to_owned()))
    );
}

#[test]
fn json_text_object_decodes_to_a_fresh_mapping() {
    let raw = JsonData::String(r#"{"ptu_count": 2, "team_id": "team-1"}"#.to_owned());
    let decoded = decode_ptu_model_info(&raw).unwrap();
    assert!(matches!(decoded, Cow::Owned(_)));
    assert_eq!(decoded.len(), 2);
}

#[rstest]
#[case::unclosed("{")]
#[case::empty_text("")]
#[case::trailing_text(r#"{"a": 1} tail"#)]
#[case::not_utf8_friendly("\u{0}")]
fn malformed_json_text_is_absent(#[case] text: &str) {
    let raw = JsonData::String(text.to_owned());
    assert!(decode_ptu_model_info(&raw).is_none());
}

#[rstest]
#[case::list("[1]")]
#[case::text_value(r#""inner""#)]
#[case::null_value("null")]
#[case::number_value("3")]
#[case::bool_value("true")]
fn decoded_nonobject_text_is_absent(#[case] text: &str) {
    let raw = JsonData::String(text.to_owned());
    assert!(decode_ptu_model_info(&raw).is_none());
}

#[rstest]
#[case::null(JsonData::Null)]
#[case::flag(JsonData::Bool(true))]
#[case::number(JsonData::Number(serde_json::Number::from(3_i32)))]
#[case::list(JsonData::Array(Vec::new()))]
fn non_text_non_object_values_are_absent(#[case] raw: JsonData) {
    assert!(decode_ptu_model_info(&raw).is_none());
}

#[test]
fn json_text_duplicate_keys_are_rejected() {
    let raw = JsonData::String(r#"{"team_id": "a", "team_id": "b"}"#.to_owned());
    assert!(decode_ptu_model_info(&raw).is_none());
}

#[test]
fn json_text_nested_past_the_depth_bound_is_absent() {
    let nested = format!("{{\"deep\": {}{}}}", "[".repeat(70), "]".repeat(70));
    let raw = JsonData::String(nested);
    assert!(decode_ptu_model_info(&raw).is_none());
}

#[test]
fn scalars_keep_each_value_kind() {
    let mapping = super::support::info(
        r#"{"off": null, "on": true, "count": 2, "name": "n", "list": [1, 2], "map": {"a": 1}}"#,
    );
    let scalars = model_info_scalars(&mapping);
    assert!(matches!(scalars.get("off"), Some(Scalar::Null)));
    assert!(matches!(scalars.get("on"), Some(Scalar::Bool(true))));
    assert!(matches!(scalars.get("count"), Some(Scalar::Number(_))));
    assert!(matches!(scalars.get("name"), Some(Scalar::Text(text)) if text == "n"));
}

#[test]
fn array_and_object_scalars_drop_their_contents() {
    let mapping = super::support::info(r#"{"list": [1, 2], "map": {"a": 1}}"#);
    let scalars = model_info_scalars(&mapping);
    assert!(matches!(scalars.get("list"), Some(Scalar::Array(items)) if items.is_empty()));
    assert!(matches!(scalars.get("map"), Some(Scalar::Object(_))));
}
