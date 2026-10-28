#![cfg(test)]

use std::error::Error as _;
use std::io;

use rstest::rstest;

use crate::otlp_errors::{CodecError, OtlpError};

#[rstest]
#[case::body_too_large(OtlpError::BodyTooLarge, "OTLP body exceeds 16777216 bytes")]
#[case::unsupported_encoding(OtlpError::UnsupportedEncoding, "Unsupported OTLP content encoding")]
#[case::invalid_gzip(
    OtlpError::InvalidGzip(io::Error::other("crc")),
    "Invalid OTLP gzip body"
)]
#[case::spans_too_large(OtlpError::SpansTooLarge("too many spans".to_owned()), "too many spans")]
#[case::invalid_spans(OtlpError::InvalidSpans("bad span".to_owned()), "bad span")]
fn displays_litellm_messages(#[case] error: OtlpError, #[case] message: &str) {
    assert_eq!(error.to_string(), message);
}

#[test]
fn invalid_gzip_keeps_its_cause() {
    let error = OtlpError::InvalidGzip(io::Error::other("crc"));
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("crc")
    );
}

#[rstest]
#[case::overflow(CodecError::Overflow("limit".to_owned()), "limit")]
#[case::malformed(CodecError::Malformed("parse".to_owned()), "parse")]
fn displays_codec_messages(#[case] error: CodecError, #[case] message: &str) {
    assert_eq!(error.to_string(), message);
}
