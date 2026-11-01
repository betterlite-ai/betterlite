#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_ocr::json::Json;
use betterlite_ocr::request_format::RequestFormat;

use super::{FormField, FormValue, OcrDocument, RawOcrRequest, parse_ocr_request};

async fn replay_corpus(dir_name: &str, content_type: &str) -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus")
        .join(dir_name);
    let mut count = 0_usize;
    for entry in fs::read_dir(&root).unwrap() {
        let body = fs::read(entry.unwrap().path()).unwrap();
        let raw = RawOcrRequest {
            content_type: Some(content_type.to_owned()),
            body,
            header_req_format: None,
            cached_form: None,
        };
        drop(parse_ocr_request(&raw).await);
        count = count.saturating_add(1);
    }
    count
}

#[tokio::test]
async fn replays_the_ocr_json_body_corpus() {
    assert!(replay_corpus("ocr_json_body", "application/json").await > 0);
}

#[tokio::test]
async fn replays_the_ocr_multipart_form_corpus() {
    assert!(replay_corpus("ocr_multipart_form", "multipart/form-data; boundary=FUZZ").await > 0);
}

#[tokio::test]
async fn json_body_is_parsed_when_content_type_is_not_multipart() {
    let raw = RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body: br#"{"model": "mistral/mistral-ocr-latest"}"#.to_vec(),
        header_req_format: None,
        cached_form: None,
    };
    let parsed = parse_ocr_request(&raw).await.unwrap();
    assert_eq!(
        parsed.fields.get("model"),
        Some(&Json::Text("mistral/mistral-ocr-latest".to_owned()))
    );
    assert_eq!(parsed.req_format, None);
}

#[tokio::test]
async fn multipart_body_is_parsed_when_content_type_is_multipart() {
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n--B--\r\n".to_vec();
    let raw = RawOcrRequest {
        content_type: Some("multipart/form-data; boundary=B".to_owned()),
        body,
        header_req_format: None,
        cached_form: None,
    };
    let parsed = parse_ocr_request(&raw).await.unwrap();
    assert!(matches!(parsed.document, Some(OcrDocument::Upload { .. })));
}

#[tokio::test]
async fn empty_body_without_cached_form_fails() {
    let raw = RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body: Vec::new(),
        header_req_format: None,
        cached_form: None,
    };
    parse_ocr_request(&raw).await.unwrap_err();
}

#[tokio::test]
async fn empty_body_with_cached_form_falls_back_to_multipart_parsing() {
    let cached_form = vec![
        FormField {
            name: "file".to_owned(),
            value: FormValue::File {
                filename: Some("a.pdf".to_owned()),
                content_type: Some("application/pdf".to_owned()),
                bytes: b"data".to_vec(),
            },
        },
        FormField {
            name: "model".to_owned(),
            value: FormValue::Text("mistral-ocr".to_owned()),
        },
    ];
    let raw = RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body: Vec::new(),
        header_req_format: None,
        cached_form: Some(cached_form),
    };
    let parsed = parse_ocr_request(&raw).await.unwrap();
    assert!(matches!(parsed.document, Some(OcrDocument::Upload { .. })));
    assert_eq!(
        parsed.fields.get("model"),
        Some(&Json::Text("mistral-ocr".to_owned()))
    );
}

#[tokio::test]
async fn req_format_header_is_used_when_body_has_none() {
    let raw = RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body: br#"{"model": "mistral/mistral-ocr-latest"}"#.to_vec(),
        header_req_format: Some("native".to_owned()),
        cached_form: None,
    };
    let parsed = parse_ocr_request(&raw).await.unwrap();
    assert_eq!(parsed.req_format, Some(RequestFormat::Native));
}

#[tokio::test]
async fn req_format_body_value_overrides_header() {
    let raw = RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body: br#"{"model": "mistral/mistral-ocr-latest", "req_format": "litellm"}"#.to_vec(),
        header_req_format: Some("native".to_owned()),
        cached_form: None,
    };
    let parsed = parse_ocr_request(&raw).await.unwrap();
    assert_eq!(parsed.req_format, Some(RequestFormat::Litellm));
}

#[tokio::test]
async fn invalid_req_format_is_rejected() {
    let raw = RawOcrRequest {
        content_type: Some("application/json".to_owned()),
        body: br#"{"req_format": "yaml"}"#.to_vec(),
        header_req_format: None,
        cached_form: None,
    };
    parse_ocr_request(&raw).await.unwrap_err();
}
