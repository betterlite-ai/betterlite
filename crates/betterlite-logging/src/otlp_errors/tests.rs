#![cfg(test)]

use crate::otlp_normalize::{OtlpError, decode_content_encoding};

#[test]
fn unsupported_encoding_is_invalid() {
    assert!(matches!(
        decode_content_encoding(b"body", Some("br")),
        Err(OtlpError::UnsupportedEncoding)
    ));
}

#[test]
fn malformed_gzip_is_invalid() {
    assert!(matches!(
        decode_content_encoding(b"not gzip", Some("gzip")),
        Err(OtlpError::InvalidGzip(_))
    ));
}
