#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::error::{SambaNovaEmbeddingError, SambaNovaError, get_error_class};
use super::{
    SUPPORTED_OPENAI_PARAMS, SambaNovaEmbeddingResponse, SambaNovaUsage,
    decode_sambanova_embedding, get_complete_url, transform_embedding_request,
    validate_environment,
};
use crate::error::ProviderError;
use crate::openai_base_config::{Headers, Object};

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::appends("https://samba.test/v1", "https://samba.test/v1/embeddings")]
#[case::trailing_slashes("https://samba.test/v1///", "https://samba.test/v1/embeddings")]
#[case::already_complete("https://samba.test/v1/embeddings", "https://samba.test/v1/embeddings")]
#[case::complete_with_slash(
    "https://samba.test/v1/embeddings/",
    "https://samba.test/v1/embeddings"
)]
#[case::empty("", "/embeddings")]
#[case::only_slashes("//", "/embeddings")]
#[case::suffix_must_be_whole(
    "https://samba.test/v1/embeddingsx",
    "https://samba.test/v1/embeddingsx/embeddings"
)]
fn url_trims_slashes_and_appends_once(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(get_complete_url(Some(base)).unwrap(), expected);
}

#[test]
fn a_missing_base_is_an_error_with_the_pinned_message() {
    let error = get_complete_url(None).unwrap_err();
    assert!(matches!(&error, SambaNovaEmbeddingError::MissingApiBase));
    assert_eq!(
        error.to_string(),
        "api_base is required for SambaNova embeddings"
    );
}

#[test]
fn headers_default_to_bearer_accept_and_json() {
    let merged = validate_environment(&Headers::new(), Some(&secret("key")), None);
    assert_eq!(
        merged,
        headers(&[
            ("Authorization", "Bearer key"),
            ("accept", "application/json"),
            ("Content-Type", "application/json"),
        ])
    );
}

#[test]
fn a_missing_key_still_writes_bearer_none() {
    let merged = validate_environment(&Headers::new(), None, None);
    assert_eq!(merged["Authorization"], "Bearer None");
}

#[test]
fn the_environment_key_is_used_only_when_no_explicit_key_is_given() {
    let merged = validate_environment(&Headers::new(), None, Some(&secret("env")));
    assert_eq!(merged["Authorization"], "Bearer env");
    let explicit =
        validate_environment(&Headers::new(), Some(&secret("key")), Some(&secret("env")));
    assert_eq!(explicit["Authorization"], "Bearer key");
}

#[test]
fn an_empty_explicit_key_does_not_fall_through_to_the_environment() {
    let merged = validate_environment(&Headers::new(), Some(&secret("")), Some(&secret("env")));
    assert_eq!(merged["Authorization"], "Bearer ");
}

#[test]
fn a_caller_authorization_replaces_the_default_one() {
    let merged = validate_environment(
        &headers(&[("Authorization", "Basic abc")]),
        Some(&secret("key")),
        None,
    );
    assert_eq!(merged["Authorization"], "Basic abc");
}

#[test]
fn other_caller_headers_are_added_and_a_differently_cased_authorization_is_a_second_entry() {
    let merged = validate_environment(
        &headers(&[("x-trace", "1"), ("authorization", "Basic abc")]),
        None,
        None,
    );
    assert_eq!(merged["x-trace"], "1");
    assert_eq!(merged["authorization"], "Basic abc");
    assert_eq!(merged["Authorization"], "Bearer None");
    assert_eq!(merged.len(), 5);
}

#[test]
fn no_embedding_param_is_supported() {
    assert!(SUPPORTED_OPENAI_PARAMS.is_empty());
}

#[test]
fn the_request_body_is_input_model_and_the_optional_params() {
    let body = transform_embedding_request(
        "sambanova/embed",
        &json!(["a", "b"]),
        &object(r#"{"dimensions": 3}"#),
    );
    assert_eq!(
        body,
        object(r#"{"input": ["a", "b"], "model": "sambanova/embed", "dimensions": 3}"#)
    );
}

#[test]
fn an_optional_param_named_model_or_input_replaces_the_written_one() {
    let body = transform_embedding_request(
        "sambanova/embed",
        &json!(["a"]),
        &object(r#"{"model": "other", "input": "text"}"#),
    );
    assert_eq!(body, object(r#"{"input": "text", "model": "other"}"#));
}

#[test]
fn a_complete_reply_keeps_model_data_object_and_usage() {
    let reply = decode_sambanova_embedding(
        br#"{"model": "m", "object": "list", "data": [{"object": "embedding", "index": 0, "embedding": [0.5]}], "usage": {"prompt_tokens": 7, "total_tokens": 9}}"#,
        200,
        &Headers::new(),
    )
    .unwrap();
    assert_eq!(reply.model.as_deref(), Some("m"));
    assert_eq!(reply.object.as_deref(), Some("list"));
    assert_eq!(reply.data.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        reply.usage,
        SambaNovaUsage {
            prompt_tokens: 7,
            total_tokens: 9,
        }
    );
}

#[test]
fn absent_reply_fields_are_null_and_the_usage_counters_default_to_zero() {
    let reply = decode_sambanova_embedding(b"{}", 200, &Headers::new()).unwrap();
    assert_eq!(reply.model, None);
    assert_eq!(reply.object, None);
    assert_eq!(reply.data, None);
    assert_eq!(reply.usage, SambaNovaUsage::default());
}

#[rstest]
#[case::prompt_only(br#"{"usage": {"prompt_tokens": 4}}"#, 4, 0)]
#[case::total_only(br#"{"usage": {"total_tokens": 6}}"#, 0, 6)]
#[case::empty_usage(br#"{"usage": {}}"#, 0, 0)]
fn each_usage_counter_defaults_on_its_own(
    #[case] body: &[u8],
    #[case] prompt: u32,
    #[case] total: u32,
) {
    let reply: SambaNovaEmbeddingResponse =
        decode_sambanova_embedding(body, 200, &Headers::new()).unwrap();
    assert_eq!(reply.usage.prompt_tokens, prompt);
    assert_eq!(reply.usage.total_tokens, total);
}

#[rstest]
#[case::not_json(b"not json", "not json")]
#[case::truncated(b"{\"model\":", "{\"model\":")]
#[case::empty(b"", "")]
fn a_body_that_is_not_json_carries_the_status_text_and_headers(
    #[case] body: &[u8],
    #[case] expected: &str,
) {
    let supplied = headers(&[("retry-after", "3")]);
    let error = decode_sambanova_embedding(body, 502, &supplied).unwrap_err();
    let SambaNovaEmbeddingError::Reply(error) = error else {
        panic!("a body that is not JSON is a SambaNovaError");
    };
    assert_eq!(error.status_code, 502);
    assert_eq!(error.message, expected);
    assert_eq!(error.headers, supplied);
}

#[rstest]
#[case::array(b"[]")]
#[case::null(b"null")]
#[case::text(b"\"reply\"")]
#[case::wrong_usage(b"{\"usage\": 3}")]
fn json_of_the_wrong_shape_is_a_typed_error(#[case] body: &[u8]) {
    assert!(matches!(
        decode_sambanova_embedding(body, 200, &Headers::new()).unwrap_err(),
        SambaNovaEmbeddingError::Shape { .. }
    ));
}

#[test]
fn the_error_class_keeps_the_message_status_and_headers() {
    let supplied = headers(&[("retry-after", "3")]);
    let error = get_error_class("busy", 429, &supplied);
    assert_eq!(
        error,
        SambaNovaError {
            status_code: 429,
            message: "busy".to_owned(),
            headers: supplied,
        }
    );
    assert_eq!(error.to_string(), "busy");
}

#[test]
fn the_error_projects_onto_the_pinned_exception_shape() {
    let supplied = headers(&[("retry-after", "3")]);
    let projected = ProviderError::from(get_error_class("busy", 429, &supplied));
    assert_eq!(projected.status_code, 429);
    assert_eq!(projected.message, "busy");
    assert_eq!(projected.request.method, "POST");
    assert_eq!(projected.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(projected.response.status_code, 429);
    assert_eq!(
        projected.headers,
        Some(vec![("retry-after".to_owned(), "3".to_owned())])
    );
    assert!(!projected.status_code_is_synthesized);
    assert_eq!(projected.body, None);
}

#[rstest]
fn replays_the_sambanova_reply_corpus(#[files("fuzz/corpus/sambanova_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _reply = decode_sambanova_embedding(&bytes, 502, &Headers::new());
}
