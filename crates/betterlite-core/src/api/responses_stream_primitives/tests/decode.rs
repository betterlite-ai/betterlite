use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::responses_stream_primitives::decode::{
    JsonShape, StreamDecodeError, decode_responses_stream_value,
};

fn decode(payload: &str) -> Json {
    decode_responses_stream_value(payload.as_bytes(), &ApiLimits::default())
        .expect("the fixture is JSON")
}

fn one(key: &str, value: Json) -> IndexMap<String, Json> {
    IndexMap::from([(key.to_owned(), value)])
}

// both loaders hand back json.loads' own value, so an array is not rejected for the object-named one
#[rstest]
#[case::an_object("{}", Json::Object(IndexMap::new()))]
#[case::an_array("[]", Json::List(Vec::new()))]
#[case::null("null", Json::Null)]
#[case::a_string("\"x\"", Json::Text("x".to_owned()))]
#[case::a_number("1", Json::Number(Number::from(1)))]
fn decode_keeps_the_payloads_shape(#[case] payload: &str, #[case] expected: Json) {
    assert_eq!(decode(payload), expected);
}

#[test]
fn decode_keeps_nested_arrays_and_objects() {
    let decoded = decode(r#"{"a": [1, null, {"b": "c"}]}"#);
    let Json::Object(fields) = decoded else {
        panic!("the payload is an object");
    };
    assert_eq!(
        fields.get("a"),
        Some(&Json::List(vec![
            Json::Number(Number::from(1)),
            Json::Null,
            Json::Object(one("b", Json::Text("c".to_owned()))),
        ]))
    );
}

// json.loads accepts these three literals and serde_json does not, which is a documented deviation
#[rstest]
#[case::a_truncated_object("{")]
#[case::an_empty_payload("")]
#[case::plain_text("not json")]
#[case::nan_literal("NaN")]
#[case::infinity_literal("Infinity")]
#[case::negative_infinity_literal("-Infinity")]
fn decode_propagates_a_json_error(#[case] payload: &str) {
    let error = decode_responses_stream_value(payload.as_bytes(), &ApiLimits::default())
        .expect_err("the fixture is not JSON");
    assert!(matches!(error, StreamDecodeError::Json { .. }));
}

#[test]
fn decode_rejects_a_payload_over_the_event_limit() {
    let limits = ApiLimits {
        max_event_bytes: 3,
        ..ApiLimits::default()
    };
    let error = decode_responses_stream_value(b"\"abc\"", &limits)
        .expect_err("the payload is over the event limit");
    assert!(matches!(
        error,
        StreamDecodeError::Limit(ApiLimitError::EventTooLarge {
            observed: 5,
            limit: 3
        })
    ));
}

#[rstest]
#[case::an_object(Json::Object(IndexMap::new()), true)]
#[case::an_array(Json::List(Vec::new()), false)]
#[case::null(Json::Null, false)]
#[case::a_string(Json::Text(String::new()), false)]
#[case::a_number(Json::Number(Number::from(1)), false)]
#[case::a_bool(Json::Bool(true), false)]
fn is_json_object_accepts_only_an_object(#[case] value: Json, #[case] expected: bool) {
    assert_eq!(value.is_json_object(), expected);
}

#[rstest]
#[case::an_array(Json::List(Vec::new()), true)]
#[case::an_object(Json::Object(IndexMap::new()), false)]
#[case::null(Json::Null, false)]
#[case::a_string(Json::Text(String::new()), false)]
fn is_json_array_accepts_only_an_array(#[case] value: Json, #[case] expected: bool) {
    assert_eq!(value.is_json_array(), expected);
}

#[rstest]
#[case::a_string(Json::Text("x".to_owned()), Some("x"))]
#[case::an_empty_string(Json::Text(String::new()), Some(""))]
#[case::a_number(Json::Number(Number::from(1)), None)]
#[case::null(Json::Null, None)]
#[case::an_object(Json::Object(IndexMap::new()), None)]
fn optional_str_keeps_only_a_string(#[case] value: Json, #[case] expected: Option<&str>) {
    assert_eq!(value.optional_str(), expected);
}

#[rstest]
#[case::an_array(Json::List(vec![Json::Null, Json::Bool(true)]), 2)]
#[case::an_object(Json::Object(IndexMap::new()), 0)]
#[case::null(Json::Null, 0)]
#[case::a_string(Json::Text("x".to_owned()), 0)]
fn json_array_or_empty_yields_items_only_for_an_array(
    #[case] value: Json,
    #[case] expected: usize,
) {
    assert_eq!(value.json_array_or_empty().len(), expected);
}

#[rstest]
#[case::an_empty_object(Json::Object(IndexMap::new()), true)]
#[case::all_values_are_strings(
    Json::Object(one("a", Json::Text("b".to_owned()))),
    true
)]
#[case::a_number_value(Json::Object(one("a", Json::Number(Number::from(1)))), false)]
#[case::an_array(Json::List(Vec::new()), false)]
fn is_str_mapping_checks_every_value(#[case] value: Json, #[case] expected: bool) {
    assert_eq!(value.is_str_mapping(), expected);
}
