#![cfg(test)]

use betterlite_ocr::json::Json;

use super::parse;
use crate::api::ocr_request::OcrDocument;

#[test]
fn accepts_a_document_url() {
    let body = br#"{"model": "mistral/mistral-ocr-latest", "document": {"type": "document_url", "document_url": "https://example.com/a.pdf"}}"#;
    let parsed = parse(body).unwrap();
    assert!(matches!(parsed.document, Some(OcrDocument::Inline(_))));
    assert_eq!(
        parsed.fields.get("model"),
        Some(&Json::Text("mistral/mistral-ocr-latest".to_owned()))
    );
    assert!(!parsed.fields.contains_key("document"));
}

#[test]
fn rejects_file_type_document() {
    let body = br#"{"document": {"type": "file", "file": "/etc/passwd"}}"#;
    parse(body).unwrap_err();
}

#[test]
fn rejects_reducto_document_url() {
    let body = br#"{"document": {"type": "document_url", "document_url": "reducto://abc"}}"#;
    parse(body).unwrap_err();
}

#[test]
fn rejects_reducto_image_url() {
    let body = br#"{"document": {"type": "image_url", "image_url": "reducto://abc"}}"#;
    parse(body).unwrap_err();
}

#[test]
fn accepts_other_document_types_and_params() {
    let body = br#"{"document": {"type": "image_url", "image_url": "https://example.com/a.png"}, "pages": [0, 1]}"#;
    let parsed = parse(body).unwrap();
    assert!(parsed.document.is_some());
    assert_eq!(
        parsed.fields.get("pages"),
        Some(&Json::List(vec![
            Json::Number(serde_json::Number::from(0_i64)),
            Json::Number(serde_json::Number::from(1_i64))
        ]))
    );
}

#[test]
fn rejects_non_object_body() {
    parse(b"[1, 2, 3]").unwrap_err();
}

#[test]
fn rejects_invalid_json() {
    parse(b"not json").unwrap_err();
}

#[test]
fn absent_document_is_allowed() {
    let body = br#"{"model": "mistral/mistral-ocr-latest"}"#;
    let parsed = parse(body).unwrap();
    assert!(parsed.document.is_none());
}
