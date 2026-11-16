#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::gemini_live_client::sdk::{
    SdkDateTime, SdkError, SdkObject, SdkValue, encode_unserializable_types, to_object,
};

use super::support::json;

const MAX_DEPTH: i64 = 100;

const fn moment(microsecond: u32, offset_seconds: Option<i32>) -> SdkValue {
    SdkValue::DateTime(SdkDateTime {
        year: 2024,
        month: 1,
        day: 2,
        hour: 3,
        minute: 4,
        second: 5,
        microsecond,
        offset_seconds,
    })
}

fn wrapped(inner: SdkValue) -> SdkObject {
    SdkObject::from([("value".to_owned(), inner)])
}

fn encoded_value(inner: SdkValue, depth: i64) -> SdkObject {
    encode_unserializable_types(wrapped(inner), depth, MAX_DEPTH)
}

fn encoded_inner(inner: SdkValue, depth: i64) -> SdkValue {
    let mut encoded = encoded_value(inner, depth);
    encoded.shift_remove("value").expect("the value survives")
}

fn object(fields: &[(&str, SdkValue)]) -> SdkValue {
    SdkValue::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

#[test]
fn bytes_become_url_safe_base64_text() {
    let encoded = encoded_inner(SdkValue::Bytes(vec![0xfb, 0xff]), 0);
    assert_eq!(encoded, SdkValue::Text("-_8=".to_owned()));
}

#[test]
fn empty_bytes_become_empty_text() {
    let encoded = encoded_inner(SdkValue::Bytes(Vec::new()), 0);
    assert_eq!(encoded, SdkValue::Text(String::new()));
}

#[rstest]
#[case::no_fraction(moment(0, None), "2024-01-02T03:04:05")]
#[case::fraction(moment(120_000, None), "2024-01-02T03:04:05.120000")]
#[case::west_of_utc(moment(0, Some(-19_800_i32)), "2024-01-02T03:04:05-05:30")]
#[case::utc(moment(0, Some(0_i32)), "2024-01-02T03:04:05+00:00")]
#[case::offset_with_seconds(moment(0, Some(18_315_i32)), "2024-01-02T03:04:05+05:05:15")]
fn datetimes_render_as_iso_text(#[case] value: SdkValue, #[case] expected: &str) {
    let encoded = encoded_inner(value, 0);
    assert_eq!(encoded, SdkValue::Text(expected.to_owned()));
}

#[test]
fn a_nested_object_is_encoded_at_the_next_depth() {
    let nested = object(&[("inner", SdkValue::Bytes(b"a".to_vec()))]);
    let encoded = encoded_inner(nested, 0);
    assert_eq!(
        encoded,
        object(&[("inner", SdkValue::Text("YQ==".to_owned()))])
    );
}

#[test]
fn a_nested_object_stays_raw_beyond_the_depth_limit() {
    let nested = object(&[("inner", SdkValue::Bytes(b"a".to_vec()))]);
    let encoded = encoded_inner(nested.clone(), MAX_DEPTH);
    assert_eq!(encoded, nested);
}

#[test]
fn a_top_level_value_at_the_depth_limit_is_encoded() {
    let encoded = encoded_value(SdkValue::Bytes(b"a".to_vec()), MAX_DEPTH);
    assert_eq!(
        encoded.get("value"),
        Some(&SdkValue::Text("YQ==".to_owned()))
    );
}

#[test]
fn a_top_level_value_beyond_the_depth_limit_is_returned_unchanged() {
    let raw = SdkValue::Bytes(b"a".to_vec());
    let encoded = encoded_value(raw.clone(), MAX_DEPTH.saturating_add(1));
    assert_eq!(encoded.get("value"), Some(&raw));
}

#[test]
fn a_list_of_datetimes_becomes_iso_text() {
    let encoded = encoded_inner(
        SdkValue::List(vec![moment(0, None), moment(120_000, None)]),
        0,
    );
    let expected = SdkValue::List(vec![
        SdkValue::Text("2024-01-02T03:04:05".to_owned()),
        SdkValue::Text("2024-01-02T03:04:05.120000".to_owned()),
    ]);
    assert_eq!(encoded, expected);
}

#[test]
fn a_list_of_bytes_stays_raw() {
    let raw = SdkValue::List(vec![
        SdkValue::Bytes(b"a".to_vec()),
        SdkValue::Bytes(b"b".to_vec()),
    ]);
    let encoded = encoded_inner(raw.clone(), 0);
    assert_eq!(encoded, raw);
}

#[test]
fn an_empty_list_stays_empty() {
    let encoded = encoded_inner(SdkValue::List(Vec::new()), 0);
    assert_eq!(encoded, SdkValue::List(Vec::new()));
}

#[test]
fn a_list_of_objects_is_encoded_item_by_item() {
    let raw = SdkValue::List(vec![object(&[("inner", SdkValue::Bytes(b"a".to_vec()))])]);
    let encoded = encoded_inner(raw, 0);
    let expected = SdkValue::List(vec![object(&[(
        "inner",
        SdkValue::Text("YQ==".to_owned()),
    )])]);
    assert_eq!(encoded, expected);
}

#[test]
fn a_list_of_lists_is_left_alone() {
    let raw = SdkValue::List(vec![SdkValue::List(vec![SdkValue::Bytes(b"a".to_vec())])]);
    let encoded = encoded_inner(raw.clone(), 0);
    assert_eq!(encoded, raw);
}

#[test]
fn a_mixed_list_is_left_alone() {
    let raw = SdkValue::List(vec![SdkValue::Bytes(b"a".to_vec()), moment(0, None)]);
    let encoded = encoded_inner(raw.clone(), 0);
    assert_eq!(encoded, raw);
}

#[test]
fn encoded_values_project_to_json() {
    let encoded = encoded_value(moment(0, Some(0_i32)), 0);
    let object = to_object(&encoded).expect("the encoded value projects");
    assert_eq!(
        Json::Object(object),
        json(r#"{"value":"2024-01-02T03:04:05+00:00"}"#)
    );
}

#[test]
fn bytes_that_survived_encoding_stop_the_json_projection() {
    let raw = SdkValue::List(vec![SdkValue::Bytes(b"a".to_vec())]);
    let encoded = encoded_value(raw, 0);
    assert!(matches!(
        to_object(&encoded),
        Err(SdkError::UnsupportedValue)
    ));
}

#[test]
fn a_datetime_that_survived_encoding_stops_the_json_projection() {
    let encoded = encoded_value(moment(0, None), MAX_DEPTH.saturating_add(1));
    assert!(matches!(
        to_object(&encoded),
        Err(SdkError::UnsupportedValue)
    ));
}

#[test]
fn a_datetime_decodes_from_its_components() {
    let decoded: SdkDateTime = serde_json::from_str(
        r#"{"year":2024,"month":1,"day":2,"hour":3,"minute":4,"second":5,"microsecond":0}"#,
    )
    .expect("the datetime decodes");
    assert_eq!(decoded.offset_seconds, None);
    let offset: SdkDateTime = serde_json::from_str(
        r#"{"year":2024,"month":1,"day":2,"hour":3,"minute":4,"second":5,"microsecond":0,"offset_seconds":-19800}"#,
    )
    .expect("the datetime decodes");
    assert_eq!(offset.offset_seconds, Some(-19_800_i32));
}
