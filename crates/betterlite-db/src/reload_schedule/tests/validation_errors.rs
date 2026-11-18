#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::budget_permission_rows::JsonData;
use crate::reload_schedule::parse_schedule::{ReloadConfigRow, parse_reload_schedule};
use crate::resource_repository::config::MAX_CONFIG_VALUE_BYTES;

fn interval_of(param_value: JsonData) -> Option<u32> {
    parse_reload_schedule(&ReloadConfigRow {
        param_value,
        last_run_at: None,
        reload_revision: Some(0),
    })
    .unwrap()
    .interval_hours
}

#[rstest]
#[case::invalid_json(r#"{"interval_hours": 8"#)]
#[case::json_null("null")]
#[case::a_json_number("5")]
#[case::a_json_array("[1]")]
#[case::an_empty_string("")]
#[case::a_textual_interval(r#"{"interval_hours": "6"}"#)]
#[case::a_float_interval(r#"{"interval_hours": 6.5}"#)]
#[case::a_bool_interval(r#"{"interval_hours": true}"#)]
#[case::an_object_interval(r#"{"interval_hours": {"hours": 6}}"#)]
#[case::a_negative_interval(r#"{"interval_hours": -5}"#)]
#[case::an_interval_past_u32(r#"{"interval_hours": 4294967296}"#)]
fn validation_errors_read_a_rejected_value_as_no_schedule(#[case] text: &str) {
    assert_eq!(interval_of(JsonData::String(text.to_owned())), None);
}

#[test]
fn validation_errors_ignore_extra_fields() {
    assert_eq!(
        interval_of(JsonData::String(
            r#"{"interval_hours": 2, "note": "x"}"#.to_owned()
        )),
        Some(2)
    );
}

#[test]
fn validation_errors_read_a_non_object_param_value_as_no_schedule() {
    assert_eq!(interval_of(JsonData::Null), None);
    assert_eq!(interval_of(JsonData::Bool(true)), None);
    assert_eq!(
        interval_of(JsonData::Number(serde_json::Number::from(6_i32))),
        None
    );
    assert_eq!(interval_of(JsonData::Array(Vec::new())), None);
    assert_eq!(
        interval_of(JsonData::Array(vec![JsonData::Number(
            serde_json::Number::from(1_i32),
        )])),
        None
    );
}

#[test]
fn validation_errors_reject_a_wrong_typed_interval_in_a_decoded_object() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "interval_hours".to_owned(),
        JsonData::String("6".to_owned()),
    );
    assert_eq!(interval_of(JsonData::Object(fields)), None);
}

#[test]
fn validation_errors_refuse_a_value_past_the_config_size_bound() {
    let padding = "x".repeat(MAX_CONFIG_VALUE_BYTES);
    let text = format!(r#"{{"interval_hours": 8, "note": "{padding}"}}"#);
    assert_eq!(interval_of(JsonData::String(text)), None);
}
