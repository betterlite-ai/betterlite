#![cfg(test)]

use super::{
    ConvertError, CostAnswer, CostCall, OcrAnswer, OcrCall, SearchCall, SessionWire,
    resolve_document,
};
use betterlite_core::search::SearchOptions;
use betterlite_models::decisions::Json;
use betterlite_ocr::json::Json as OcrJson;
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_input::OcrDocumentInput;
use betterlite_ocr::response::OCRResponse;
use std::collections::BTreeMap;

fn ocr_json(text: &str) -> OcrJson {
    serde_json::from_str(text).expect("the document is JSON")
}

#[test]
fn request_search_call_reads_every_public_keyword() {
    let text = r#"{"query": ["one", "two"], "search_provider": "brave", "max_results": 5, "search_domain_filter": ["example.com"], "max_tokens_per_page": 512, "country": "US", "api_key": "sk-test", "api_base": "http://127.0.0.1:8080", "timeout_seconds": 2.5, "extra_headers": {"x-extra": "one"}, "kwargs": {"objective": "find docs"}}"#;
    let call: SearchCall = serde_json::from_str(text).expect("the call parses");
    let request = call.into_request().expect("the request converts");
    assert_eq!(request.search_provider, "brave");
    assert_eq!(
        request.query,
        Json::List(vec![
            Json::Text("one".to_owned()),
            Json::Text("two".to_owned())
        ])
    );
    assert_eq!(
        request.options,
        SearchOptions {
            max_results: Some(5),
            search_domain_filter: Some(vec!["example.com".to_owned()]),
            max_tokens_per_page: Some(512),
            country: Some("US".to_owned()),
        }
    );
    assert!(request.api_key.is_some());
}

#[test]
fn request_search_call_keeps_the_connection_and_the_kwargs() {
    let text = r#"{"query": "docs", "search_provider": "brave", "api_base": "http://127.0.0.1:8080", "timeout_seconds": 2.5, "extra_headers": {"x-extra": "one"}, "kwargs": {"objective": "find docs"}}"#;
    let call: SearchCall = serde_json::from_str(text).expect("the call parses");
    let request = call.into_request().expect("the request converts");
    assert_eq!(
        request.extra_headers.get("x-extra").map(String::as_str),
        Some("one")
    );
    assert_eq!(
        request.timeout.map(|value| value.as_secs_f64()),
        Some(2.5_f64)
    );
    assert_eq!(
        request.kwargs.get("objective"),
        Some(&Json::Text("find docs".to_owned()))
    );
}

#[test]
fn request_search_call_defaults_every_option_to_absent() {
    let text = r#"{"query": "docs", "search_provider": "brave"}"#;
    let call: SearchCall = serde_json::from_str(text).expect("the call parses");
    let request = call.into_request().expect("the request converts");
    assert_eq!(request.options, SearchOptions::default());
    assert!(request.extra_headers.is_empty());
    assert!(request.kwargs.is_empty());
    assert!(request.timeout.is_none());
    assert!(request.api_key.is_none());
}

#[test]
fn request_search_call_rejects_a_negative_timeout() {
    let text = r#"{"query": "docs", "search_provider": "brave", "timeout_seconds": -1.0}"#;
    let call: SearchCall = serde_json::from_str(text).expect("the call parses");
    let error = call.into_request().expect_err("the timeout is refused");
    assert!(matches!(error, ConvertError::Timeout { .. }));
}

#[test]
fn request_search_call_rejects_an_unknown_keyword() {
    let text = r#"{"query": "docs", "search_provider": "brave", "unknown": true}"#;
    let error = serde_json::from_str::<SearchCall>(text).expect_err("the field is refused");
    assert!(error.to_string().contains("unknown field `unknown`"));
}

#[test]
fn request_cost_call_defaults_to_one_query_and_no_params() {
    let call: CostCall =
        serde_json::from_str(r#"{"model": "exa_ai/search"}"#).expect("the call parses");
    assert_eq!(call.number_of_queries, 1_i64);
    assert!(call.custom_llm_provider.is_none());
    assert!(call.optional_params.max_results.is_none());
}

#[test]
fn request_cost_call_reads_the_tiered_max_results() {
    let text = r#"{"model": "exa_ai/search", "custom_llm_provider": "exa_ai", "number_of_queries": 3, "optional_params": {"max_results": 12}}"#;
    let call: CostCall = serde_json::from_str(text).expect("the call parses");
    assert_eq!(call.number_of_queries, 3_i64);
    assert_eq!(call.custom_llm_provider.as_deref(), Some("exa_ai"));
    assert_eq!(call.optional_params.max_results, Some(12));
}

#[test]
fn request_ocr_call_reads_the_document_and_connection() {
    let text = r#"{"model": "mistral-ocr-latest", "document": {"type": "document_url", "document_url": "data:application/pdf;base64,YWJj"}, "api_key": "sk-test", "api_base": "http://127.0.0.1:8080", "custom_llm_provider": "mistral", "extra_headers": {"x-extra": "one"}, "optional_params": {"pages": [0]}, "input_sources": {"api_key": "request"}, "timeout_seconds": 4.0}"#;
    let call: OcrCall = serde_json::from_str(text).expect("the call parses");
    assert_eq!(call.model, "mistral-ocr-latest");
    assert_eq!(call.custom_llm_provider.as_deref(), Some("mistral"));
    assert_eq!(call.timeout_seconds, Some(4.0_f64));
    assert_eq!(
        call.input_sources.get("api_key").map(String::as_str),
        Some("request")
    );
    assert!(call.optional_params.contains_key("pages"));
    assert_eq!(
        call.extra_headers
            .as_ref()
            .and_then(|headers| headers.get("x-extra"))
            .cloned(),
        Some(OcrJson::Text("one".to_owned()))
    );
}

#[test]
fn request_document_url_decodes_through_the_core_validation() {
    let document =
        ocr_json(r#"{"type": "document_url", "document_url": "https://example.com/a.pdf"}"#);
    let resolved = resolve_document(&document, &[]).expect("the url document decodes");
    assert_eq!(
        resolved,
        OcrDocumentInput::Document(OcrDocument::DocumentUrl {
            document_url: "https://example.com/a.pdf".to_owned(),
            extra: BTreeMap::new(),
        })
    );
}

#[test]
fn request_file_document_reads_its_bytes_from_the_blobs() {
    let document = ocr_json(
        r#"{"type": "file", "file": {"$file": {"name": "file", "blob": 0, "content_type": "application/pdf"}}}"#,
    );
    let resolved =
        resolve_document(&document, &[b"%PDF-1.4".to_vec()]).expect("the file document decodes");
    assert_eq!(
        resolved,
        OcrDocumentInput::Bytes {
            bytes: b"%PDF-1.4".to_vec(),
            file_name: None,
            mime_type: Some("application/pdf".to_owned()),
        }
    );
}

#[test]
fn request_file_document_keeps_a_named_stream_and_its_mime_type() {
    let document = ocr_json(
        r#"{"type": "file", "mime_type": "image/png", "file": {"$file": {"name": "scan.png", "blob": 1, "content_type": "application/pdf"}}}"#,
    );
    let blobs = vec![b"first".to_vec(), b"second".to_vec()];
    let resolved = resolve_document(&document, &blobs).expect("the file document decodes");
    assert_eq!(
        resolved,
        OcrDocumentInput::Bytes {
            bytes: b"second".to_vec(),
            file_name: Some("scan.png".to_owned()),
            mime_type: Some("image/png".to_owned()),
        }
    );
}

#[test]
fn error_file_document_without_a_file_names_the_missing_field() {
    let document = ocr_json(r#"{"type": "file"}"#);
    let error = resolve_document(&document, &[]).expect_err("the document is refused");
    assert_eq!(
        error.to_string(),
        "invalid OCR request: document with type='file' must include a 'file' field containing a pathlib.Path, file-like object, or bytes"
    );
}

#[test]
fn error_file_document_refuses_a_bare_string() {
    let document = ocr_json(r#"{"type": "file", "file": "scan.pdf"}"#);
    let error = resolve_document(&document, &[]).expect_err("the document is refused");
    assert!(matches!(error, OcrError::InvalidRequest(_)));
    assert!(error.to_string().contains("bare str"));
}

#[test]
fn error_file_document_refuses_a_marker_without_bytes() {
    let document = ocr_json(
        r#"{"type": "file", "file": {"$file": {"name": "file", "blob": 2, "content_type": null}}}"#,
    );
    let error =
        resolve_document(&document, &[b"only".to_vec()]).expect_err("the document is refused");
    assert!(matches!(error, OcrError::InvalidRequest(_)));
}

#[test]
fn error_document_without_a_type_names_the_field() {
    let document = ocr_json("{}");
    let error = resolve_document(&document, &[]).expect_err("the document is refused");
    assert_eq!(
        error.to_string(),
        "invalid OCR request field: document.type"
    );
}

#[test]
fn request_session_wire_reads_an_absent_environment_as_none() {
    let wire: SessionWire = serde_json::from_str("{}").expect("the wire parses");
    assert!(wire.environment.is_none());
    let named: SessionWire =
        serde_json::from_str(r#"{"environment": {"MISTRAL_API_KEY": "sk-test"}}"#)
            .expect("the wire parses");
    assert_eq!(
        named
            .environment
            .as_ref()
            .and_then(|env| env.get("MISTRAL_API_KEY"))
            .map(String::as_str),
        Some("sk-test")
    );
}

// req_format=native hands the provider's own payload beside the normalized response, and Python reads it by that key
#[test]
fn request_ocr_answer_keeps_the_provider_native_response() {
    let response: OCRResponse =
        serde_json::from_str(r#"{"pages": [], "model": "mistral-ocr-latest"}"#)
            .expect("the response decodes");
    let native = ocr_json(r#"{"pages": [], "model": "mistral-ocr-latest"}"#);
    let text = serde_json::to_string(&OcrAnswer {
        response: &response,
        provider_native_response: Some(&native),
    })
    .expect("the answer encodes");
    let encoded: OcrJson = serde_json::from_str(&text).expect("the answer is JSON");
    assert!(matches!(
        &encoded,
        OcrJson::Object(fields) if fields.get("provider_native_response") == Some(&native)
    ));
}

#[test]
fn request_cost_answer_tags_the_unmapped_failure() {
    let text = serde_json::to_string(&CostAnswer::Unmapped {
        message: "model isn't mapped yet".to_owned(),
    })
    .expect("the answer encodes");
    assert_eq!(
        text,
        r#"{"outcome":"unmapped","message":"model isn't mapped yet"}"#
    );
}
