#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;

use super::error::{VolcEngineEmbeddingError, get_error_class};
use super::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::reply::decode_reply;
use super::request::{get_complete_url, transform_embedding_request, validate_environment};
use crate::embedding::{EmbeddingInput, EmbeddingResponse, EmbeddingUsage, EmbeddingVector};
use crate::openai_base_config::{Headers, Object};

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn header_map(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn map(params: &str, options: &str, drop_params: bool) -> Result<Object, VolcEngineEmbeddingError> {
    let mut optional = object(options);
    map_openai_params(&object(params), &mut optional, drop_params).map(|()| optional)
}

#[test]
fn the_supported_list_is_the_fixed_litellm_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["encoding_format", "user", "extra_headers"]
    );
}

#[rstest]
#[case::float(r#"{"encoding_format": "float"}"#)]
#[case::base64(r#"{"encoding_format": "base64"}"#)]
#[case::null(r#"{"encoding_format": null}"#)]
#[case::user(r#"{"user": "u"}"#)]
#[case::null_user(r#"{"user": null}"#)]
#[case::extra_headers(r#"{"extra_headers": {"x-call": "1"}}"#)]
fn a_supported_option_is_kept_as_given(#[case] params: &str) {
    assert_eq!(map(params, "{}", false).unwrap(), object(params));
}

#[rstest]
#[case::empty(r#"{"encoding_format": ""}"#)]
#[case::wrong_case(r#"{"encoding_format": "Float"}"#)]
#[case::number(r#"{"encoding_format": 1}"#)]
#[case::bool(r#"{"encoding_format": true}"#)]
#[case::list(r#"{"encoding_format": ["float"]}"#)]
fn a_bad_encoding_format_fails_without_drop_params(#[case] params: &str) {
    assert!(matches!(
        map(params, "{}", false),
        Err(VolcEngineEmbeddingError::UnsupportedEncodingFormat { .. })
    ));
}

#[rstest]
#[case::empty(r#"{"encoding_format": ""}"#)]
#[case::number(r#"{"encoding_format": 1}"#)]
#[case::bool(r#"{"encoding_format": true}"#)]
fn a_bad_encoding_format_is_dropped_with_drop_params(#[case] params: &str) {
    assert_eq!(map(params, "{}", true).unwrap(), Object::new());
}

#[rstest]
#[case::dimensions(r#"{"dimensions": 3}"#)]
#[case::modalities(r#"{"modalities": ["text"]}"#)]
fn an_unsupported_option_fails_without_drop_params(#[case] params: &str) {
    assert!(matches!(
        map(params, "{}", false),
        Err(VolcEngineEmbeddingError::UnsupportedParam { .. })
    ));
}

#[rstest]
#[case::dimensions(r#"{"dimensions": 3}"#)]
#[case::modalities(r#"{"modalities": ["text"]}"#)]
fn an_unsupported_option_is_dropped_with_drop_params(#[case] params: &str) {
    assert_eq!(map(params, "{}", true).unwrap(), Object::new());
}

#[test]
fn the_options_already_present_survive_the_mapping() {
    let mapped = map(
        r#"{"encoding_format": "float"}"#,
        r#"{"dimensions": 9, "other": 1}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        mapped,
        object(r#"{"dimensions": 9, "other": 1, "encoding_format": "float"}"#)
    );
}

#[test]
fn the_error_names_the_rejected_option() {
    let error = map(r#"{"dimensions": 3}"#, "{}", false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unsupported parameter for Volcengine: dimensions"
    );
    let format = map(r#"{"encoding_format": 1}"#, "{}", false).unwrap_err();
    assert!(
        format
            .to_string()
            .starts_with("Unsupported encoding_format: 1.")
    );
}

#[rstest]
#[case::default(None, "https://ark.cn-beijing.volces.com/api/v3/embeddings")]
#[case::default_given(
    Some("https://ark.cn-beijing.volces.com"),
    "https://ark.cn-beijing.volces.com/api/v3/embeddings"
)]
#[case::already_versioned(
    Some("https://gw.example.test/api/v3"),
    "https://gw.example.test/api/v3/embeddings"
)]
#[case::trailing_slash_is_not_versioned(
    Some("https://gw.example.test/api/v3/"),
    "https://gw.example.test/api/v3//api/v3/embeddings"
)]
#[case::plain_base(
    Some("https://gw.example.test/"),
    "https://gw.example.test//api/v3/embeddings"
)]
fn the_url_adds_the_version_segment_unless_it_is_there(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(get_complete_url(api_base), expected);
}

#[test]
fn the_environment_needs_a_key() {
    let error = validate_environment(&Headers::new(), None).unwrap_err();
    assert!(matches!(error, VolcEngineEmbeddingError::MissingApiKey));
}

#[rstest]
#[case::plain("volc-key", "Bearer volc-key")]
#[case::empty("", "Bearer ")]
fn the_environment_adds_the_volcengine_headers(#[case] key: &str, #[case] expected: &str) {
    let headers = validate_environment(&Headers::new(), Some(&secret(key))).unwrap();
    assert_eq!(
        headers,
        header_map(&[
            ("Authorization", expected),
            ("Content-Type", "application/json"),
        ])
    );
}

#[test]
fn the_volcengine_headers_win_over_a_caller_header_with_the_same_spelling() {
    let caller = header_map(&[
        ("Content-Type", "text/plain"),
        ("Authorization", "Custom"),
        ("content-type", "kept"),
        ("x-call", "1"),
    ]);
    let headers = validate_environment(&caller, Some(&secret("volc-key"))).unwrap();
    assert_eq!(
        headers,
        header_map(&[
            ("Authorization", "Bearer volc-key"),
            ("Content-Type", "application/json"),
            ("content-type", "kept"),
            ("x-call", "1"),
        ])
    );
}

#[rstest]
#[case::single_text(EmbeddingInput::Text(String::from("hello")), r#"["hello"]"#)]
#[case::texts(
    EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]),
    r#"["a", "b"]"#
)]
#[case::tokens(EmbeddingInput::Tokens(vec![1, 2]), "[1, 2]")]
#[case::token_lists(EmbeddingInput::TokenLists(vec![vec![1, 2]]), "[[1, 2]]")]
fn a_single_string_becomes_a_one_item_list_and_lists_stay(
    #[case] input: EmbeddingInput,
    #[case] expected: &str,
) {
    let body = transform_embedding_request("doubao-embedding", &input, &Object::new());
    assert_eq!(body.get("input"), Some(&value(expected)));
    assert_eq!(
        body.get("model").and_then(Value::as_str),
        Some("doubao-embedding")
    );
}

#[test]
fn only_the_two_body_options_are_added_and_a_null_one_is_left_out() {
    let options = object(
        r#"{"encoding_format": "base64", "user": "u", "extra_headers": {"x-call": "1"}, "other": 1}"#,
    );
    let body =
        transform_embedding_request("m", &EmbeddingInput::Text(String::from("hi")), &options);
    assert_eq!(
        body,
        object(r#"{"model": "m", "input": ["hi"], "encoding_format": "base64", "user": "u"}"#)
    );
    let nulls = object(r#"{"encoding_format": null, "user": null}"#);
    let null_body =
        transform_embedding_request("m", &EmbeddingInput::Text(String::from("hi")), &nulls);
    assert_eq!(null_body, object(r#"{"model": "m", "input": ["hi"]}"#));
}

fn reply(text: &str) -> EmbeddingResponse {
    serde_json::from_str(text).unwrap()
}

#[test]
fn the_reply_is_built_as_the_openai_embedding_response() {
    let raw = r#"{"id": "dropped", "object": "not-used", "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}], "model": "m1", "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}}"#;
    let decoded = decode_reply(raw.as_bytes(), "req-model").unwrap();
    assert_eq!(
        decoded,
        reply(
            r#"{"object": "list", "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}], "model": "m1", "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4, "completion_tokens_details": null, "prompt_tokens_details": null}}"#
        )
    );
}

#[rstest]
#[case::missing_model(r#"{"data": []}"#, "req-model")]
#[case::null_data(r#"{"data": null, "model": "m2"}"#, "m2")]
#[case::missing_data(r#"{"model": "m2"}"#, "m2")]
fn the_reply_falls_back_to_the_request_model_and_an_empty_data_list(
    #[case] raw: &str,
    #[case] model: &str,
) {
    let decoded = decode_reply(raw.as_bytes(), "req-model").unwrap();
    assert_eq!(decoded.model, model);
    assert!(decoded.data.is_empty());
    assert_eq!(decoded.object, "list");
}

#[test]
fn the_reply_usage_defaults_every_missing_counter_to_zero() {
    let decoded = decode_reply(br#"{"data": [], "usage": null}"#, "m").unwrap();
    assert_eq!(decoded.usage, usage(0, 0, 0));
    let sparse = decode_reply(br#"{"data": [], "usage": {"prompt_tokens": 5}}"#, "m").unwrap();
    assert_eq!(sparse.usage, usage(5, 0, 0));
}

fn usage(prompt_tokens: u32, completion_tokens: u32, total_tokens: u32) -> EmbeddingUsage {
    EmbeddingUsage {
        prompt_tokens,
        completion_tokens,
        completion_tokens_details: None,
        prompt_tokens_details: None,
        total_tokens,
    }
}

#[rstest]
#[case::not_json("not json at all")]
#[case::root_list("[1, 2]")]
#[case::data_not_a_list(r#"{"data": "nope"}"#)]
#[case::item_without_index(r#"{"data": [{"object": "embedding", "embedding": [1]}]}"#)]
#[case::null_model(r#"{"data": [], "model": null}"#)]
#[case::numeric_model(r#"{"data": [], "model": 7}"#)]
#[case::usage_not_an_object(r#"{"data": [], "usage": 7}"#)]
fn a_malformed_reply_is_a_typed_error_with_status_500(#[case] raw: &str) {
    let error = decode_reply(raw.as_bytes(), "m").unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.headers.is_empty());
    assert!(!error.message.is_empty());
}

#[test]
fn a_base64_embedding_stays_a_string() {
    let decoded = decode_reply(
        br#"{"data": [{"object": "embedding", "embedding": "Zm9v", "index": 0}], "model": "m"}"#,
        "m",
    )
    .unwrap();
    assert_eq!(
        decoded.data.first().map(|record| record.embedding.clone()),
        Some(EmbeddingVector::Base64(String::from("Zm9v")))
    );
}

#[test]
fn the_error_class_keeps_the_status_the_message_and_the_headers() {
    let supplied = header_map(&[("retry-after", "2")]);
    let error = get_error_class("rate limited", 429, &supplied);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rate limited");
    assert_eq!(error.headers, supplied);
    assert!(
        get_error_class("boom", 500, &Headers::new())
            .headers
            .is_empty()
    );
}

#[rstest]
fn replays_the_volcengine_embedding_reply_corpus(
    #[files("fuzz/corpus/volcengine_embedding_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _reply = decode_reply(&bytes, "replay-model");
}
