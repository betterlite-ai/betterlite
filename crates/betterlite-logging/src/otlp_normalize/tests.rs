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
    let span = NormalizedSpan {
        attributes: [
            ("used".to_owned(), "x".to_owned()),
            ("kept".to_owned(), "y".to_owned()),
        ]
        .into_iter()
        .collect(),
        consumed_attributes: vec!["used".to_owned()],
        ..NormalizedSpan::default()
    };
    assert!(!span_row(&span).span_attributes.contains_key("used"));
}

#[test]
fn replays_the_otlp_decode_corpus() {
    for body in [b"".as_slice(), b"{}".as_slice(), b"[]".as_slice()] {
        drop(decode_otlp(body, None, None));
    }
}
