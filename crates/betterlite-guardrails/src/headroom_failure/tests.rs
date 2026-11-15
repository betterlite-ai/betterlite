#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use super::{
    CompressFailure, FAILURE_STATUS, NOT_FOUND_HINT, build_compress_failure_detail,
    handle_compress_failure, read_ccr_hashes,
};
use crate::headroom_settings::UnreachableFallback;
use crate::primitives::json_value::JsonValue;

fn rows(source: &str) -> Vec<JsonValue> {
    serde_json::from_str(source).unwrap()
}

fn value(source: &str) -> JsonValue {
    serde_json::from_str(source).unwrap()
}

fn text(value: &JsonValue) -> &str {
    let JsonValue::String(text) = value else {
        panic!("not a string: {value:?}");
    };
    text
}

#[test]
fn failure_detail_adds_the_hint_for_a_404_only() {
    let detail = build_compress_failure_detail(404, "not found");
    assert_eq!(
        detail.get("status_code"),
        Some(&JsonValue::Number(serde_json::Number::from(404_u16)))
    );
    assert_eq!(detail.get("body").map(text), Some("not found"));
    assert_eq!(detail.get("hint").map(text), Some(NOT_FOUND_HINT));
    assert_eq!(detail.len(), 3);
}

#[rstest]
#[case::server_error(500)]
#[case::bad_request(400)]
fn failure_detail_keeps_only_status_and_body_for_other_statuses(#[case] status: u16) {
    let detail = build_compress_failure_detail(status, "broken");
    assert_eq!(
        detail.get("status_code"),
        Some(&JsonValue::Number(serde_json::Number::from(status)))
    );
    assert_eq!(detail.get("body").map(text), Some("broken"));
    assert!(!detail.contains_key("hint"));
    assert_eq!(detail.len(), 2);
}

#[test]
fn fail_open_returns_the_rows_unchanged() {
    let messages = rows(r#"[{"role": "user", "content": "u"}]"#);
    let detail = build_compress_failure_detail(500, "broken");
    let forwarded = handle_compress_failure(
        UnreachableFallback::FailOpen,
        &messages,
        "Headroom compression service returned an error",
        &detail,
    )
    .unwrap();
    assert_eq!(forwarded, messages);
}

#[test]
fn fail_closed_returns_a_502_body_with_the_error_and_the_detail() {
    let detail = build_compress_failure_detail(404, "not found");
    let failure: CompressFailure = handle_compress_failure(
        UnreachableFallback::FailClosed,
        &[],
        "Headroom compression service returned an error",
        &detail,
    )
    .unwrap_err();
    assert_eq!(failure.status, FAILURE_STATUS);
    assert_eq!(failure.status, 502);
    let mut expected = IndexMap::new();
    expected.insert(
        "error".to_owned(),
        JsonValue::String("Headroom compression service returned an error".to_owned()),
    );
    expected.extend(detail);
    assert_eq!(failure.body, expected);
}

#[test]
fn ccr_hashes_are_lowercased_filtered_and_kept_once() {
    let body = value(
        r#"{"ccr_hashes": [
            "ABCDEF012345",
            "abcdef012345",
            "FEDCBA987654",
            "0123456789ab",
            "0123456789abc",
            "0123456789abcdef01234567",
            "0123456789abcdef012345678",
            "0123456789a",
            "zzzzzzzzzzzz",
            5,
            null
        ]}"#,
    );
    let hashes: Vec<String> = read_ccr_hashes(&body).into_iter().collect();
    assert_eq!(
        hashes,
        vec![
            "0123456789ab".to_owned(),
            "0123456789abc".to_owned(),
            "0123456789abcdef01234567".to_owned(),
            "abcdef012345".to_owned(),
            "fedcba987654".to_owned()
        ]
    );
}

#[rstest]
#[case::not_a_list(r#"{"ccr_hashes": "abcdef012345"}"#)]
#[case::missing(r#"{"messages": []}"#)]
#[case::object(r#"{"ccr_hashes": {"hash": "abcdef012345"}}"#)]
fn ccr_hashes_are_empty_without_a_list(#[case] body: &str) {
    assert!(read_ccr_hashes(&value(body)).is_empty());
}

#[test]
fn ccr_hashes_are_empty_for_a_body_that_is_not_an_object() {
    assert!(read_ccr_hashes(&JsonValue::Null).is_empty());
}
