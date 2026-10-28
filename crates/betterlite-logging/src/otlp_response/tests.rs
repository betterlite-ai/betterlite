#![cfg(test)]

use rstest::rstest;

use super::encode_otlp_response;

// Longer than 127 bytes, so its protobuf length takes two varint bytes
const LONG_ERROR: &str = "OTLP body exceeds 16777216 bytes; the collector should batch spans into smaller export requests before sending them to this proxy endpoint";

#[rstest]
#[case::json_success(Some("application/json"), None, b"{}".to_vec(), "application/json")]
#[case::json_empty_error(Some("application/json"), Some(""), b"{}".to_vec(), "application/json")]
#[case::json_error(
    Some(" Application/JSON ; charset=utf-8"),
    Some("bad \"span\" \u{e9}"),
    concat!(r#"{"message": "bad \"span\" "#, "\\", r#"u00e9"}"#).as_bytes().to_vec(),
    "application/json"
)]
#[case::default_success(None, None, Vec::new(), "application/x-protobuf")]
#[case::empty_type_success(Some(""), None, Vec::new(), "application/x-protobuf")]
#[case::protobuf_empty_error(
    Some("application/x-protobuf"),
    Some(""),
    Vec::new(),
    "application/x-protobuf"
)]
#[case::protobuf_error(Some("text/plain"), Some("boom"), b"\x12\x04boom".to_vec(), "application/x-protobuf")]
#[case::long_protobuf_error(
    None,
    Some(LONG_ERROR),
    [b"\x12\x8a\x01".as_slice(), LONG_ERROR.as_bytes()].concat(),
    "application/x-protobuf"
)]
fn encodes_like_litellm(
    #[case] content_type: Option<&str>,
    #[case] error: Option<&str>,
    #[case] body: Vec<u8>,
    #[case] media_type: &str,
) {
    assert_eq!(
        encode_otlp_response(content_type, error),
        (body, media_type)
    );
}
