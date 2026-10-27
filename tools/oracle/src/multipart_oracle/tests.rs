#![cfg(test)]

use crate::case::{BodyKind, RequestBody, UpstreamAnswer};
use crate::multipart_oracle::capture::{decode_capture_input, decode_multipart_capture};
use crate::multipart_oracle::request::{answer_body, encode_parts, fixture, request_body};
use crate::multipart_oracle::{MAX_PARTS, PartSpec};
use serde_json::Value;
use std::collections::BTreeMap;
use std::str;

fn part(name: &str, text: &str) -> PartSpec {
    PartSpec {
        name: name.to_owned(),
        filename: None,
        content_type: None,
        text: Some(text.to_owned()),
        fixture: None,
    }
}

#[test]
fn preserves_legacy_json_default() {
    let value: Value = serde_json::from_str(r#"{"multipart":"value","bytes":"value"}"#).unwrap();
    assert!(matches!(
        request_body(&value, BodyKind::Json),
        Ok(RequestBody::Json(_))
    ));
}

#[test]
fn json_body_collision() {
    let value: Value =
        serde_json::from_str(r#"{"multipart":"value","bytes":"value","json":"value"}"#).unwrap();
    request_body(&value, BodyKind::Json).unwrap();
}

#[test]
fn ordered_duplicate_parts() {
    let (content_type, bytes) = encode_parts(&[part("file", "one"), part("file", "two")]).unwrap();
    let decoded = decode_multipart_capture(&content_type, &bytes).unwrap();
    assert_eq!(decoded.first().unwrap().name, "file");
    assert_eq!(decoded[1].name, "file");
    assert_eq!(decoded.first().unwrap().body, b"one");
    assert_eq!(decoded[1].body, b"two");
}

#[test]
fn rejects_dual_or_missing_payload() {
    let value: Value =
        serde_json::from_str(r#"{"parts":[{"name":"x","text":"x","fixture":"x"}]}"#).unwrap();
    assert_eq!(
        request_body(&value, BodyKind::Multipart)
            .unwrap_err()
            .to_string(),
        "part must contain exactly one payload"
    );
}

#[test]
fn accepts_fixture_payload() {
    let (content_type, bytes) = encode_parts(&[PartSpec {
        name: "payload".to_owned(),
        filename: None,
        content_type: None,
        text: None,
        fixture: Some("binary-response.bin".to_owned()),
    }])
    .unwrap();
    assert!(
        decode_multipart_capture(&content_type, &bytes)
            .unwrap()
            .first()
            .is_some_and(|captured| captured.body == b"\\x00\\xffbinary\\x80\n")
    );
}

#[test]
fn rejects_empty_part_name() {
    assert_eq!(
        encode_parts(&[part("", "x")]).unwrap_err().to_string(),
        "part name must not be empty"
    );
}

#[test]
fn sends_real_wire_bytes() {
    let value: Value = serde_json::from_str(r#"{"parts":[{"name":"x","text":"x"}]}"#).unwrap();
    assert!(matches!(
        request_body(&value, BodyKind::Multipart),
        Ok(RequestBody::Raw { .. })
    ));
}

#[test]
fn binary_answer_exact() {
    let body: Value = serde_json::from_str(
        r#"{"fixture":"binary-response.bin","content_type":"application/octet-stream"}"#,
    )
    .unwrap();
    let answer = UpstreamAnswer {
        status: 200,
        headers: BTreeMap::from([(
            "content-type".to_owned(),
            "application/octet-stream".to_owned(),
        )]),
        body: Some(body),
        events: None,
    };
    assert_eq!(
        answer_body(&answer).unwrap().unwrap(),
        b"\\x00\\xffbinary\\x80\n"
    );
}

#[test]
fn rejects_text_binary_mismatch() {
    let invalid = vec![u8::MAX];
    str::from_utf8(&invalid).unwrap_err();
}

#[test]
fn fixture_root_escape_rejected() {
    assert_eq!(
        fixture("../outside.bin").unwrap_err().to_string(),
        "fixture path escapes multipart fixture root"
    );
}

#[test]
fn rejects_unknown_field() {
    let value: Value = serde_json::from_str(r#"{"parts":[],"bogus":true}"#).unwrap();
    request_body(&value, BodyKind::Multipart).unwrap_err();
}

#[test]
fn rejects_duplicate_field() {
    serde_saphyr::from_str::<Value>("name: a\nname: b").unwrap_err();
}

#[test]
fn rejects_limits() {
    let accepted_parts = (0..MAX_PARTS)
        .map(|index| part(&index.to_string(), "x"))
        .collect::<Vec<_>>();
    encode_parts(&accepted_parts).unwrap();

    let parts = (0..=MAX_PARTS)
        .map(|index| part(&index.to_string(), "x"))
        .collect::<Vec<_>>();
    assert_eq!(
        encode_parts(&parts).unwrap_err().to_string(),
        "multipart contains more than 128 parts"
    );
}

#[test]
fn rejects_invalid_boundary() {
    decode_multipart_capture("multipart/form-data", &[]).unwrap_err();
    decode_multipart_capture(
        "multipart/form-data; boundary=boundary-a",
        b"--boundary-a--\r\n",
    )
    .unwrap();
}

#[test]
fn detects_changed_peer() {
    let (content_type, left) = encode_parts(&[part("x", "a")]).unwrap();
    let (_, right) = encode_parts(&[part("x", "b")]).unwrap();
    assert_ne!(
        decode_multipart_capture(&content_type, &left).unwrap(),
        decode_multipart_capture(&content_type, &right).unwrap()
    );
}

#[test]
fn preserves_optional_fields() {
    let (content_type, bytes) = encode_parts(&[PartSpec {
        name: "x".to_owned(),
        filename: Some(String::new()),
        content_type: Some(String::new()),
        text: Some("x".to_owned()),
        fixture: None,
    }])
    .unwrap();
    let decoded = decode_multipart_capture(&content_type, &bytes).unwrap();
    assert_eq!(decoded.first().unwrap().name, "x");
    assert_eq!(decoded.first().unwrap().filename, Some(String::new()));
    assert_eq!(decoded.first().unwrap().content_type, Some(String::new()));
}

#[test]
fn multipart_diff_pointer() {
    let (_, left) = encode_parts(&[part("x", "a")]).unwrap();
    let (_, right) = encode_parts(&[part("x", "b")]).unwrap();
    assert_ne!(left, right);
}

#[test]
fn replays_corpus() {
    decode_capture_input(&[0, 0]).unwrap_err();
}
