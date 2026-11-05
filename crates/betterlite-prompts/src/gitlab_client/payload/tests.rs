#![cfg(test)]

use rstest::rstest;

use super::{PayloadError, decode_file_payload, decode_text};

#[rstest]
#[case::ascii(b"abc".as_slice(), "abc")]
#[case::unicode("é✓".as_bytes(), "é✓")]
#[case::lone_invalid_byte(b"a\xffb".as_slice(), "a\u{FFFD}b")]
#[case::truncated_sequence(b"a\xe2\x82".as_slice(), "a\u{FFFD}")]
#[case::empty(b"".as_slice(), "")]
fn decode_text_replaces_invalid_utf8(#[case] bytes: &[u8], #[case] expected: &str) {
    assert_eq!(decode_text(bytes), expected);
}

#[rstest]
#[case::padded(r#"{"content": "YWJj", "encoding": "base64"}"#, Some("abc"))]
#[case::unpadded_tail(r#"{"content": "YQ==", "encoding": "base64"}"#, Some("a"))]
#[case::non_canonical_trailing_bits(r#"{"content": "YR==", "encoding": "base64"}"#, Some("a"))]
#[case::garbage_characters_dropped(r#"{"content": "Y W-J_j!", "encoding": "base64"}"#, Some("abc"))]
#[case::other_encoding(r#"{"content": "YWJj", "encoding": "text"}"#, Some("YWJj"))]
#[case::null_content(r#"{"content": null, "encoding": "base64"}"#, None)]
#[case::no_content("{}", None)]
fn decode_file_payload_reads_the_content_field(#[case] body: &str, #[case] expected: Option<&str>) {
    let decoded = decode_file_payload(body.as_bytes()).unwrap();
    assert_eq!(decoded.as_deref(), expected);
}

#[test]
fn invalid_json_is_a_json_error() {
    assert!(matches!(
        decode_file_payload(b"[1]"),
        Err(PayloadError::Json(_))
    ));
}

#[test]
fn undecodable_base64_is_a_base64_error() {
    let outcome = decode_file_payload(br#"{"content": "A", "encoding": "base64"}"#);
    assert!(matches!(outcome, Err(PayloadError::Base64(_))));
}
