#![cfg(test)]

use super::*;

#[test]
fn identity_body_is_unchanged() {
    assert_eq!(decode_content_encoding(b"abc", None).unwrap(), b"abc");
}

#[test]
fn oversized_attribute_has_byte_marker() {
    let output = truncate(&"x".repeat(OTLP_MAX_ATTRIBUTE_VALUE_BYTES + 1));
    assert!(output.contains("[truncated 1 bytes]"));
}

#[test]
fn consumed_attributes_are_excluded() {
    let span: NormalizedSpan = serde_json::from_value(serde_json::json!({
        "attributes": {"used": "x", "kept": "y"},
        "consumed_attributes": ["used"]
    }))
    .unwrap();
    assert!(!span_row(&span).span_attributes.contains_key("used"));
}

#[test]
fn replays_the_otlp_decode_corpus() {
    drop(decode_otlp(b"", None, None));
    drop(decode_otlp(b"{}", None, None));
    drop(decode_otlp(b"[]", None, None));
}
