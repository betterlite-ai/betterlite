#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::video_records::helpers::video_reference_to_id;

use super::fixture::{assert_not_text, object};

#[test]
fn video_reference_reads_an_id_from_an_object() {
    let cases = [
        (
            object(&[("id", Json::Text("video_123".to_owned()))]),
            "video_123",
        ),
        (object(&[("object", Json::Text("video".to_owned()))]), ""),
        (object(&[("id", Json::Text(String::new()))]), ""),
        (IndexMap::new(), ""),
    ];
    for (fields, expected) in cases {
        assert_eq!(
            video_reference_to_id(&Json::Object(fields)).unwrap(),
            Some(expected.to_owned())
        );
    }
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

#[test]
fn video_reference_decodes_a_json_string() {
    let cases = [
        (r#"{"id": "video_456"}"#, "video_456"),
        (r#"{"object": "video"}"#, ""),
        (r#"{"id": ""}"#, ""),
        ("42", "42"),
        ("[]", "[]"),
        (r#""video_789""#, r#""video_789""#),
        ("null", "null"),
        ("true", "true"),
        ("", ""),
    ];
    for (text, expected) in cases {
        assert_eq!(
            video_reference_to_id(&Json::Text(text.to_owned())).unwrap(),
            Some(expected.to_owned()),
            "{text:?}"
        );
    }
}

#[test]
fn video_reference_keeps_malformed_json() {
    let cases = [
        ("video_123", "video_123"),
        ("  spaced id  ", "  spaced id  "),
        ("{oops", "{oops"),
        (r#"{"id": "#, r#"{"id": "#),
        (r#"{"id": 1,}"#, r#"{"id": 1,}"#),
    ];
    for (text, expected) in cases {
        assert_eq!(
            video_reference_to_id(&Json::Text(text.to_owned())).unwrap(),
            Some(expected.to_owned()),
            "{text:?}"
        );
    }
}

#[test]
fn video_reference_returns_empty_for_other_types() {
    let cases = [
        Json::Null,
        Json::Bool(true),
        Json::Number(Number::from(7_i64)),
        Json::List(vec![Json::Text("video_1".to_owned())]),
    ];
    for value in cases {
        assert_eq!(
            video_reference_to_id(&value).unwrap(),
            Some(String::new()),
            "{value:?}"
        );
    }
}

#[test]
fn video_reference_rejects_a_non_text_object_id() {
    let cases = [
        Json::Bool(true),
        Json::Number(Number::from(5_i64)),
        Json::List(Vec::new()),
        Json::Object(IndexMap::new()),
    ];
    for entry in cases {
        let error = video_reference_to_id(&Json::Object(object(&[("id", entry)]))).unwrap_err();
        assert_not_text(error, "id");
    }
}

#[test]
fn video_reference_rejects_a_non_text_id_inside_a_json_string() {
    let error = video_reference_to_id(&Json::Text(r#"{"id": 5}"#.to_owned())).unwrap_err();
    assert_not_text(error, "id");
}
