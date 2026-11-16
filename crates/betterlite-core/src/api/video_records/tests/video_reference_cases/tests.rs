#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::video_records::helpers::video_reference_to_id;

use super::fixture::{assert_not_text, object};

#[rstest]
#[case::id(object(&[("id", Json::Text("video_123".to_owned()))]), "video_123")]
#[case::without_id(object(&[("object", Json::Text("video".to_owned()))]), "")]
#[case::empty_id(object(&[("id", Json::Text(String::new()))]), "")]
#[case::empty_object(IndexMap::<String, Json>::new(), "")]
fn video_reference_reads_an_id_from_an_object(
    #[case] fields: IndexMap<String, Json>,
    #[case] expected: &str,
) {
    assert_eq!(
        video_reference_to_id(&Json::Object(fields)).unwrap(),
        Some(expected.to_owned())
    );
}

#[test]
fn video_reference_hands_back_a_null_id() {
    assert_eq!(
        video_reference_to_id(&Json::Object(object(&[("id", Json::Null)]))).unwrap(),
        None
    );
    assert_eq!(
        video_reference_to_id(&Json::Text(r#"{"id": null}"#.to_owned())).unwrap(),
        None
    );
}

#[rstest]
#[case::object_id(r#"{"id": "video_456"}"#, "video_456")]
#[case::object_without_id(r#"{"object": "video"}"#, "")]
#[case::empty_object_id(r#"{"id": ""}"#, "")]
#[case::number("42", "42")]
#[case::list("[]", "[]")]
#[case::json_string(r#""video_789""#, r#""video_789""#)]
#[case::null("null", "null")]
#[case::boolean("true", "true")]
#[case::empty("", "")]
fn video_reference_decodes_a_json_string(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(
        video_reference_to_id(&Json::Text(text.to_owned())).unwrap(),
        Some(expected.to_owned()),
        "{text:?}"
    );
}

#[rstest]
#[case::plain_id("video_123", "video_123")]
#[case::spaced_id("  spaced id  ", "  spaced id  ")]
#[case::truncated_object("{oops", "{oops")]
#[case::unfinished_value(r#"{"id": "#, r#"{"id": "#)]
#[case::trailing_comma(r#"{"id": 1,}"#, r#"{"id": 1,}"#)]
fn video_reference_keeps_malformed_json(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(
        video_reference_to_id(&Json::Text(text.to_owned())).unwrap(),
        Some(expected.to_owned()),
        "{text:?}"
    );
}

#[rstest]
#[case::null(Json::Null)]
#[case::bool(Json::Bool(true))]
#[case::number(Json::Number(Number::from(7_i64)))]
#[case::list(Json::List(vec![Json::Text("video_1".to_owned())]))]
fn video_reference_returns_empty_for_other_types(#[case] value: Json) {
    assert_eq!(
        video_reference_to_id(&value).unwrap(),
        Some(String::new()),
        "{value:?}"
    );
}

#[rstest]
#[case::bool(Json::Bool(true))]
#[case::number(Json::Number(Number::from(5_i64)))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn video_reference_rejects_a_non_text_object_id(#[case] entry: Json) {
    let error = video_reference_to_id(&Json::Object(object(&[("id", entry)]))).unwrap_err();
    assert_not_text(error, "id");
}

#[test]
fn video_reference_rejects_a_non_text_id_inside_a_json_string() {
    let error = video_reference_to_id(&Json::Text(r#"{"id": 5}"#.to_owned())).unwrap_err();
    assert_not_text(error, "id");
}
