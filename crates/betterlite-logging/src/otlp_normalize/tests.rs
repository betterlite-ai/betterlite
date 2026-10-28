#![cfg(test)]

use super::*;

#[test]
fn identity_body_is_unchanged() {
    assert_eq!(decode_content_encoding(b"abc", None).unwrap(), b"abc");
    assert_eq!(
        decode_content_encoding(b"abc", Some("identity")).unwrap(),
        b"abc"
    );
}

#[test]
fn unsupported_and_oversized_bodies_are_rejected() {
    assert!(matches!(
        decode_content_encoding(b"abc", Some("br")),
        Err(OtlpError::UnsupportedEncoding)
    ));
    let body = vec![0_u8; OTLP_MAX_BODY_BYTES + 1];
    assert!(matches!(
        decode_content_encoding(&body, None),
        Err(OtlpError::TooLarge { .. })
    ));
}

#[test]
fn gzip_body_is_decoded_and_gzip_overflow_is_rejected() {
    use std::io::Write;

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"hello").unwrap();
    let compressed = encoder.finish().unwrap();
    assert_eq!(
        decode_content_encoding(&compressed, Some("gzip")).unwrap(),
        b"hello"
    );

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&vec![b'x'; OTLP_MAX_BODY_BYTES + 1])
        .unwrap();
    let compressed = encoder.finish().unwrap();
    assert!(matches!(
        decode_content_encoding(&compressed, Some("gzip")),
        Err(OtlpError::TooLarge { .. })
    ));
}

#[test]
fn oversized_attribute_has_byte_marker() {
    let output = truncate(&"x".repeat(OTLP_MAX_ATTRIBUTE_VALUE_BYTES + 1));
    assert!(output.contains("[truncated 1 bytes]"));
}

#[test]
fn truncation_ignores_incomplete_utf8() {
    let value = format!("{}é", "x".repeat(OTLP_MAX_ATTRIBUTE_VALUE_BYTES - 1));
    let output = truncate(&value);
    assert!(output.ends_with("…[truncated 2 bytes]"));
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
fn span_row_uses_exception_and_response_ids() {
    let span: NormalizedSpan = serde_json::from_value(serde_json::json!({
        "start_ns": 10,
        "end_ns": 20,
        "status_message": "",
        "attributes": {"gen_ai.response.id": "response"},
        "events": [{"name":"exception", "attributes":{"exception.type":"failure"}}]
    }))
    .unwrap();
    let row = span_row(&span);
    assert_eq!(row.duration, 10);
    assert_eq!(row.status_message, "failure");
    assert_eq!(row.litellm_request_id, "response");
}

#[test]
fn replays_the_otlp_decode_corpus() {
    drop(decode_otlp(b"", None, None));
    drop(decode_otlp(b"{}", None, None));
    drop(decode_otlp(b"[]", None, None));
}

#[test]
fn decodes_a_valid_span_payload() {
    let rows = decode_otlp(br#"[{"trace_id":"trace","span_id":"span"}]"#, None, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trace_id, "trace");
}
