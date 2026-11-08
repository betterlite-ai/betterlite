#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{TextResult, ToolOutcome, WireCompat, parse_http_body, wire_compat_for};

#[rstest]
#[case::modern("2026-07-28", WireCompat::Modern)]
#[case::nov_2025("2025-11-25", WireCompat::Legacy)]
#[case::nov_2024("2024-11-05", WireCompat::Legacy)]
#[case::unknown("2099-01-01", WireCompat::Legacy)]
fn wire_compat_uses_the_modern_revision_only(#[case] version: &str, #[case] expected: WireCompat) {
    assert_eq!(wire_compat_for(version), expected);
}

#[rstest]
#[case::empty("")]
#[case::whitespace("   \n\t")]
#[case::invalid("{not json")]
#[case::json_null("null")]
fn a_body_that_is_not_a_json_value_stays_text(#[case] body: &str) {
    assert_eq!(
        parse_http_body(body),
        ToolOutcome::Text(TextResult {
            text: body.to_owned()
        })
    );
}

#[rstest]
#[case::object(r#"{"a": 1}"#)]
#[case::array("[1, 2]")]
#[case::number("42")]
#[case::text("\"hi\"")]
#[case::boolean("true")]
fn a_json_body_keeps_its_value_and_original_text(#[case] body: &str) {
    let ToolOutcome::Json(result) = parse_http_body(body) else {
        panic!("a JSON body must parse to a JsonResult");
    };
    assert_eq!(result.value, serde_json::from_str::<Json>(body).unwrap());
    assert_eq!(result.original_text, body);
}

#[rstest]
fn a_json_object_body_keeps_its_keys() {
    let ToolOutcome::Json(result) = parse_http_body(r#"{"name": "value"}"#) else {
        panic!("a JSON object body must parse to a JsonResult");
    };
    let Json::Object(object) = result.value else {
        panic!("the value must stay an object");
    };
    assert_eq!(object.get("name"), Some(&Json::Text("value".to_owned())));
}

#[rstest]
fn a_blank_body_keeps_its_original_whitespace() {
    let ToolOutcome::Text(result) = parse_http_body(" \n ") else {
        panic!("a blank body must stay text");
    };
    assert_eq!(result.text, " \n ");
}
