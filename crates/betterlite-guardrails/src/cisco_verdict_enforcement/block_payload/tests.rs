#![cfg(test)]

use crate::cisco_conversation::json::Json;
use crate::cisco_verdict_enforcement::block_payload::error_fields_from_detail;

#[test]
fn error_fields_from_detail_passes_an_object_through_unchanged() {
    let detail = Json::parse(r#"{"error": "blocked", "message": "nope"}"#);
    let fields = error_fields_from_detail(&detail);
    assert_eq!(
        fields.get("error").map(|raw| raw.get()),
        Some("\"blocked\"")
    );
    assert_eq!(fields.get("message").map(|raw| raw.get()), Some("\"nope\""));
}

#[test]
fn error_fields_from_detail_wraps_a_non_object_detail_as_a_message() {
    let detail = Json::parse(r#""plain text""#);
    let fields = error_fields_from_detail(&detail);
    assert_eq!(
        fields.get("message").map(|raw| raw.get()),
        Some("\"plain text\"")
    );
}
