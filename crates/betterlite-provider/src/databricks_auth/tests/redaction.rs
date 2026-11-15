use std::collections::BTreeMap;

use crate::databricks_auth::Headers;
use crate::databricks_auth::redaction::{redact_headers_for_logging, redact_sensitive_data};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde_json::json;

fn text(value: &str) -> Json {
    serde_json::from_value(json!(value)).expect("a text fixture parses")
}

fn redacted(value: &str) -> String {
    match redact_sensitive_data(&text(value)) {
        Json::Text(text) => text,
        other => panic!("a text stays text, found {other:?}"),
    }
}

#[rstest]
#[case::bearer("Bearer abc123def", "Bearer [REDACTED]")]
#[case::authorization("Authorization: Bearer xyz", "Authorization: [REDACTED] [REDACTED]")]
fn a_token_text_loses_its_value(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(redacted(input), expected);
}

#[test]
fn the_api_key_assignment_keeps_its_closing_quote() {
    assert_eq!(redacted("api_key=\"abcd\""), "api_key=\"[REDACTED]\"");
}

#[test]
fn a_client_secret_assignment_is_redacted() {
    assert_eq!(
        redacted("client_secret='xyz'"),
        "client_secret='[REDACTED]'"
    );
}

#[test]
fn a_source_api_key_is_replaced_whole() {
    let token = format!("dapi{}", "a".repeat(32));
    assert_eq!(redacted(&token), "[REDACTED_PAT]");
}

#[test]
fn the_access_token_assignment_is_redacted() {
    assert_eq!(
        redacted("\"access_token\": \"abc\""),
        "\"access_token\": \"[REDACTED]\""
    );
}

#[test]
fn a_short_dapi_word_stays() {
    assert_eq!(redacted("dapiabc"), "dapiabc");
}

#[test]
fn a_field_names_a_secret_even_when_its_value_is_not_a_string() {
    let value: Json = serde_json::from_value(json!({"access_token": {"nested": "Bearer x"}}))
        .expect("a fixture parses");
    assert_eq!(
        redact_sensitive_data(&value),
        serde_json::from_value::<Json>(json!({"access_token": "[REDACTED]"})).expect("expected")
    );
}

#[test]
fn a_key_matches_sensitive_parts_as_a_substring() {
    let value: Json =
        serde_json::from_value(json!({"max_tokens": 5, "note": "plain"})).expect("a fixture");
    assert_eq!(
        redact_sensitive_data(&value),
        serde_json::from_value::<Json>(json!({"max_tokens": "[REDACTED]", "note": "plain"}))
            .expect("expected")
    );
}

#[test]
fn a_list_is_walked_in_order() {
    let value: Json = serde_json::from_value(json!(["Bearer a", 1, null])).expect("a fixture");
    assert_eq!(
        redact_sensitive_data(&value),
        serde_json::from_value::<Json>(json!(["Bearer [REDACTED]", 1, null])).expect("expected")
    );
}

#[test]
fn an_empty_header_map_stays_empty() {
    assert_eq!(redact_headers_for_logging(&Headers::new()), Headers::new());
}

#[rstest]
#[case::long("Bearer abcdefghijkl", "Bearer a...[REDACTED]")]
#[case::exactly_ten("0123456789", "[REDACTED]")]
#[case::eleven("01234567890", "01234567...[REDACTED]")]
fn a_sensitive_header_shows_its_first_eight(#[case] value: &str, #[case] expected: &str) {
    let headers = BTreeMap::from([("Authorization".to_owned(), value.to_owned())]);
    let redacted = redact_headers_for_logging(&headers);
    assert_eq!(
        redacted.get("Authorization").map(String::as_str),
        Some(expected)
    );
}

#[rstest]
#[case::api_key("x-api-key")]
#[case::api_key_short("Api-Key")]
#[case::databricks("x-databricks-token")]
#[case::authorization("AUTHORIZATION")]
fn a_header_name_is_matched_without_case(#[case] name: &str) {
    let headers = BTreeMap::from([
        (name.to_owned(), "a-long-secret-value".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ]);
    let redacted = redact_headers_for_logging(&headers);
    assert_eq!(
        redacted.get(name).map(String::as_str),
        Some("a-long-s...[REDACTED]")
    );
    assert_eq!(
        redacted.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}
