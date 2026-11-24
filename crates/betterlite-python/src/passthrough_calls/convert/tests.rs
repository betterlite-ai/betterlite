#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Object;
use secrecy::ExposeSecret as _;
use serde_json::json;
use std::collections::BTreeMap;

use super::{
    API_ERROR, BAD_REQUEST, BAD_REQUEST_STATUS, ConvertError, ErrorPayload, SYNTHESIZED_STATUS,
    call_args, encode,
};

const FULL_REQUEST: &str = r#"{
    "method": "POST",
    "endpoint": "chat/completions",
    "model": "vllm/my-model",
    "custom_llm_provider": "vllm",
    "api_base": "http://127.0.0.1:9/v1",
    "api_key": "sk-test",
    "request_query_params": {"b": "2", "a": "1"},
    "request_headers": {"x-one": "1", "x-two": "2"},
    "content": {"$file": {"name": "file", "blob": 0, "content_type": null}},
    "data": {"field": "value"},
    "files": {"image": {"$file": {"name": "cat.png", "blob": 1, "content_type": "image/png"}}},
    "json": {"model": "original"},
    "params": [["p", "1"]],
    "cookies": [["session", "s1"]],
    "stream": true,
    "timeout": 5
}"#;

fn full_blobs() -> Vec<Vec<u8>> {
    vec![b"pong".to_vec(), b"png-bytes".to_vec()]
}

#[test]
fn an_answer_that_cannot_be_written_as_json_is_an_encode_error() {
    let value = BTreeMap::from([(vec![1_u8], "value")]);
    let error = encode(&value).unwrap_err();
    assert!(matches!(error, ConvertError::Encode { .. }));
}

#[test]
fn request_reads_every_named_parameter() {
    let mut blobs = full_blobs();
    let args = call_args(FULL_REQUEST, &mut blobs).unwrap();
    assert_eq!(args.method, "POST");
    assert_eq!(args.endpoint, "chat/completions");
    assert_eq!(args.model, "vllm/my-model");
    assert_eq!(args.custom_llm_provider.as_deref(), Some("vllm"));
    assert_eq!(args.api_base.as_deref(), Some("http://127.0.0.1:9/v1"));
    let key = args.api_key.unwrap();
    assert_eq!(key.expose_secret(), "sk-test");
    assert_eq!(args.content.as_deref(), Some(&b"pong"[..]));
    assert_eq!(args.data.unwrap().get("field"), Some(&json!("value")));
    assert_eq!(args.json.unwrap().get("model"), Some(&json!("original")));
    assert_eq!(
        args.params.unwrap(),
        vec![(String::from("p"), String::from("1"))]
    );
    assert_eq!(
        args.cookies.unwrap(),
        vec![(String::from("session"), String::from("s1"))]
    );
}

#[test]
fn request_keeps_the_order_of_query_parameters_and_headers() {
    let mut blobs = full_blobs();
    let args = call_args(FULL_REQUEST, &mut blobs).unwrap();
    let query: Vec<(String, String)> = args.request_query_params.unwrap().into_iter().collect();
    assert_eq!(
        query,
        vec![
            (String::from("b"), String::from("2")),
            (String::from("a"), String::from("1"))
        ]
    );
    assert_eq!(
        args.request_headers.unwrap(),
        vec![
            (String::from("x-one"), String::from("1")),
            (String::from("x-two"), String::from("2"))
        ]
    );
}

#[test]
fn request_lifts_the_file_markers_into_a_multipart_upload() {
    let mut blobs = full_blobs();
    let args = call_args(FULL_REQUEST, &mut blobs).unwrap();
    let uploads = args.files.unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].field, "image");
    assert_eq!(uploads[0].filename, "cat.png");
    assert_eq!(uploads[0].content_type.as_deref(), Some("image/png"));
    assert_eq!(uploads[0].bytes.as_ref(), b"png-bytes");
    // both blobs were moved out of the caller's list, so nothing is sent twice
    assert_eq!(blobs.len(), 2);
    assert!(blobs[0].is_empty());
    assert!(blobs[1].is_empty());
}

#[test]
fn request_starts_from_no_client_and_the_loose_kwargs() {
    let request = r#"{"method": "GET", "endpoint": "models", "model": "vllm/m",
        "stream": true, "metadata": {"k": "v"}}"#;
    let args = call_args(request, &mut Vec::new()).unwrap();
    assert!(args.client.is_none());
    assert_eq!(args.extra.get("stream"), Some(&json!(true)));
    let metadata: JsonValue = serde_json::from_str(r#"{"k": "v"}"#).unwrap();
    assert_eq!(args.extra.get("metadata"), Some(&metadata));
    assert!(args.content.is_none());
    assert!(args.files.is_none());
    assert!(args.data.is_none());
    assert!(args.request_query_params.is_none());
}

#[test]
fn request_reads_a_named_field_only_once() {
    let request = r#"{"method": "POST", "endpoint": "e", "model": "m", "endpoint": "twice"}"#;
    assert!(matches!(
        call_args(request, &mut Vec::new()),
        Err(ConvertError::Json { .. })
    ));
}

#[test]
fn request_refuses_a_content_that_is_not_a_file_marker() {
    let request = r#"{"method": "POST", "endpoint": "e", "model": "m", "content": "text"}"#;
    assert!(matches!(
        call_args(request, &mut Vec::new()),
        Err(ConvertError::Content)
    ));
}

#[test]
fn request_refuses_a_file_marker_with_no_blob() {
    let request = r#"{"method": "POST", "endpoint": "e", "model": "m",
        "files": {"image": {"$file": {"name": "a.png", "blob": 3}}}}"#;
    assert!(matches!(
        call_args(request, &mut Vec::new()),
        Err(ConvertError::File { key: "files", .. })
    ));
}

#[test]
fn request_refuses_a_blank_text() {
    assert!(matches!(
        call_args("", &mut Vec::new()),
        Err(ConvertError::Json { .. })
    ));
}

fn provider_error(status: u16, body: Option<Object>) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code: status,
        message: String::from("Invalid API key"),
        headers: Some(vec![(String::from("x-request-id"), String::from("r1"))]),
        request: None,
        response: None,
        body,
        status_code_is_synthesized: false,
    })
}

#[test]
fn error_provider_keeps_the_status_message_and_body() {
    let body: JsonValue = serde_json::from_str(r#"{"error": {"message": "bad key"}}"#).unwrap();
    let payload = ErrorPayload::provider(
        &provider_error(401, body.as_object().cloned()),
        "vllm",
        "vllm/m",
    );
    let text = serde_json::to_value(&payload).unwrap();
    assert_eq!(text["type"], json!("APIError"));
    assert_eq!(text["status_code"], json!(401_i32));
    assert_eq!(text["message"], json!("Invalid API key"));
    assert_eq!(text["llm_provider"], json!("vllm"));
    assert_eq!(text["model"], json!("vllm/m"));
    assert_eq!(text["body"], body);
    let expected_headers: JsonValue = serde_json::from_str(r#"{"x-request-id": "r1"}"#).unwrap();
    assert_eq!(text["headers"], expected_headers);
}

#[test]
fn error_upstream_reports_the_status_headers_and_body() {
    let headers = vec![(
        String::from("content-type"),
        String::from("application/json"),
    )];
    let payload = ErrorPayload::upstream(404, &headers, br#"{"error": "missing"}"#);
    assert_eq!(payload.kind, "APIError");
    assert_eq!(payload.status_code, Some(404));
    assert_eq!(payload.message, r#"{"error": "missing"}"#);
    let expected_body: JsonValue = serde_json::from_str(r#"{"error": "missing"}"#).unwrap();
    assert_eq!(payload.body, Some(expected_body));
    assert_eq!(payload.headers.unwrap()["content-type"], "application/json");
}

#[test]
fn error_upstream_keeps_a_non_json_body_as_text_only() {
    let payload = ErrorPayload::upstream(500, &[], b"not json");
    assert_eq!(payload.message, "not json");
    assert!(payload.body.is_none());
}

#[test]
fn error_upstream_names_a_body_that_is_not_utf8() {
    let payload = ErrorPayload::upstream(500, &[], &[0xff, 0xfe]);
    assert_eq!(
        payload.message,
        "the upstream answered with 2 bytes of non-UTF-8 body"
    );
    assert!(payload.body.is_none());
}

#[test]
fn error_resolution_is_a_bad_request() {
    let payload = ErrorPayload::failed(
        BAD_REQUEST,
        Some(BAD_REQUEST_STATUS),
        String::from("LLM Provider NOT provided"),
    );
    assert_eq!(payload.kind, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.message, "LLM Provider NOT provided");
}

#[test]
fn error_provider_not_found_is_a_plain_exception() {
    let payload = ErrorPayload::provider_not_found("acme");
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.message, "Provider acme not found");
    assert!(payload.status_code.is_none());
}

#[test]
fn error_routing_keeps_the_router_status() {
    let payload = ErrorPayload::routing(429, "slow down");
    assert_eq!(payload.kind, "APIError");
    assert_eq!(payload.status_code, Some(429));
    assert_eq!(payload.message, "slow down");
    assert!(payload.headers.is_none());
}

#[test]
fn error_transport_is_a_synthesized_500() {
    let payload = ErrorPayload::failed(
        API_ERROR,
        Some(SYNTHESIZED_STATUS),
        String::from("connection refused"),
    );
    assert_eq!(payload.kind, "APIError");
    assert_eq!(payload.status_code, Some(500));
    assert_eq!(payload.message, "connection refused");
    assert!(payload.body.is_none());
    assert!(payload.headers.is_none());
}
