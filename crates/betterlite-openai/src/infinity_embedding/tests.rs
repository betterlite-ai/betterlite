#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::embedding_host_settings::Settings;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;

use super::error::{InfinityEmbeddingError, SYNTHETIC_REQUEST_URL, get_error_class};
use super::{
    InfinityUsage, SUPPORTED_OPENAI_PARAMS, decode_infinity_embedding, get_complete_url,
    map_openai_params, transform_embedding_request, validate_environment,
};
use crate::openai_base_config::{Headers, Object};

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
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

fn settings(key: Option<&str>) -> Settings {
    Settings {
        jina_ai_api_base: None,
        jina_ai_api_key: Some(secret("jina")),
        infinity_api_key: key.map(secret),
    }
}

#[rstest]
#[case::appends("http://infinity.test", "http://infinity.test/embeddings")]
#[case::trailing_slashes("http://infinity.test///", "http://infinity.test/embeddings")]
#[case::already_complete("http://infinity.test/embeddings", "http://infinity.test/embeddings")]
#[case::complete_with_slash("http://infinity.test/embeddings/", "http://infinity.test/embeddings")]
#[case::empty("", "/embeddings")]
#[case::only_slashes("//", "/embeddings")]
#[case::suffix_must_be_whole(
    "http://infinity.test/v1/embeddingsx",
    "http://infinity.test/v1/embeddingsx/embeddings"
)]
fn url_trims_slashes_and_appends_once(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(get_complete_url(Some(base)).unwrap(), expected);
}

#[test]
fn a_missing_base_is_rejected() {
    assert!(matches!(
        get_complete_url(None),
        Err(InfinityEmbeddingError::MissingApiBase)
    ));
}

#[rstest]
#[case::explicit(Some("explicit"), Some("env"), "Bearer explicit")]
#[case::environment(None, Some("env"), "Bearer env")]
#[case::absent_is_the_word_none(None, None, "Bearer None")]
#[case::explicit_empty_is_kept(Some(""), Some("env"), "Bearer ")]
fn the_key_falls_back_to_infinity_api_key_only(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] authorization: &str,
) {
    let supplied = explicit.map(secret);
    let merged = validate_environment(&Headers::new(), supplied.as_ref(), &settings(environment));
    assert_eq!(
        merged,
        headers(&[
            ("Authorization", authorization),
            ("accept", "application/json"),
            ("Content-Type", "application/json"),
        ])
    );
}

#[rstest]
#[case::authorization(&[("Authorization", "Bearer mine")], &[("Authorization", "Bearer mine"), ("accept", "application/json"), ("Content-Type", "application/json")])]
#[case::accept(&[("accept", "text/plain")], &[("Authorization", "Bearer key"), ("accept", "text/plain"), ("Content-Type", "application/json")])]
#[case::content_type(&[("Content-Type", "text/plain")], &[("Authorization", "Bearer key"), ("accept", "application/json"), ("Content-Type", "text/plain")])]
#[case::extra(&[("X-Extra", "1")], &[("Authorization", "Bearer key"), ("accept", "application/json"), ("Content-Type", "application/json"), ("X-Extra", "1")])]
fn caller_headers_override_every_default(
    #[case] supplied: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    let key = secret("key");
    assert_eq!(
        validate_environment(&headers(supplied), Some(&key), &settings(None)),
        headers(expected)
    );
}

#[test]
fn dimensions_is_renamed_and_the_other_two_pass_through() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["encoding_format", "modality", "dimensions"]
    );
    let mut optional = Object::new();
    let requested = object(
        r#"{
        "encoding_format": "base64", "modality": "image", "dimensions": 128, "user": "u"
    }"#,
    );
    map_openai_params(&requested, &mut optional);
    assert_eq!(
        optional,
        object(r#"{"encoding_format": "base64", "modality": "image", "output_dimension": 128}"#)
    );
}

#[rstest]
#[case::encoding_format(r#"{"encoding_format": "float"}"#, r#"{"encoding_format": "float"}"#)]
#[case::modality(r#"{"modality": "text"}"#, r#"{"modality": "text"}"#)]
#[case::dimensions(r#"{"dimensions": 8}"#, r#"{"output_dimension": 8}"#)]
#[case::none("{}", "{}")]
fn each_parameter_maps_on_its_own(#[case] requested: &str, #[case] expected: &str) {
    let mut optional = Object::new();
    map_openai_params(&object(requested), &mut optional);
    assert_eq!(optional, object(expected));
}

#[test]
fn options_overlay_input_and_model() {
    let plain = transform_embedding_request("bge", &parse(r#"["a"]"#), &Object::new());
    assert_eq!(plain, object(r#"{"input": ["a"], "model": "bge"}"#));
    let options = object(r#"{"input": ["b"], "model": "other", "output_dimension": 4}"#);
    let overlaid = transform_embedding_request("bge", &parse(r#"["a"]"#), &options);
    assert_eq!(
        overlaid,
        object(r#"{"input": ["b"], "model": "other", "output_dimension": 4}"#)
    );
}

#[test]
fn the_input_is_forwarded_untouched() {
    let body = transform_embedding_request("bge", &parse(r#""scalar""#), &Object::new());
    assert_eq!(body.get("input"), Some(&parse(r#""scalar""#)));
}

#[test]
fn the_reply_keeps_each_field_and_both_counters() {
    let raw = br#"{"model":"bge","object":"list","data":[{"object":"embedding","embedding":[0.5],"index":0}],"usage":{"prompt_tokens":4,"total_tokens":9}}"#;
    let reply = decode_infinity_embedding(raw, 200).unwrap();
    assert_eq!(reply.model.as_deref(), Some("bge"));
    assert_eq!(reply.object.as_deref(), Some("list"));
    assert_eq!(reply.data.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        reply.usage,
        InfinityUsage {
            prompt_tokens: 4,
            total_tokens: 9
        }
    );
}

#[rstest]
#[case::no_usage("{}", 0, 0)]
#[case::null_usage(r#"{"usage":null}"#, 0, 0)]
#[case::empty_usage(r#"{"usage":{}}"#, 0, 0)]
#[case::only_prompt(r#"{"usage":{"prompt_tokens":5}}"#, 5, 0)]
#[case::only_total(r#"{"usage":{"total_tokens":6}}"#, 0, 6)]
#[case::null_counters(r#"{"usage":{"prompt_tokens":null,"total_tokens":null}}"#, 0, 0)]
#[case::different_counters(r#"{"usage":{"prompt_tokens":7,"total_tokens":3}}"#, 7, 3)]
fn usage_counters_default_independently(
    #[case] raw: &str,
    #[case] prompt_tokens: u32,
    #[case] total_tokens: u32,
) {
    let reply = decode_infinity_embedding(raw.as_bytes(), 200).unwrap();
    assert_eq!(
        reply.usage,
        InfinityUsage {
            prompt_tokens,
            total_tokens
        }
    );
}

#[test]
fn absent_and_null_fields_stay_absent() {
    let absent = decode_infinity_embedding(b"{}", 200).unwrap();
    assert_eq!(
        (absent.model, absent.data, absent.object),
        (None, None, None)
    );
    let raw = br#"{"model":null,"data":null,"object":null}"#;
    let nulls = decode_infinity_embedding(raw, 200).unwrap();
    assert_eq!((nulls.model, nulls.data, nulls.object), (None, None, None));
}

#[rstest]
#[case::truncated(b"{\"model\"", 502, "{\"model\"")]
#[case::text(b"upstream down", 503, "upstream down")]
#[case::empty(b"", 200, "")]
fn invalid_json_is_wrapped_with_the_upstream_status(
    #[case] raw: &[u8],
    #[case] status: u16,
    #[case] text: &str,
) {
    let error = decode_infinity_embedding(raw, status).unwrap_err();
    assert!(matches!(
        &error,
        InfinityEmbeddingError::Json { status_code, message, .. }
            if *status_code == status && message == text
    ));
}

#[test]
fn bytes_that_are_not_utf8_fall_back_to_the_decoder_message() {
    let error = decode_infinity_embedding(b"\xff\xfe", 500).unwrap_err();
    assert!(matches!(
        &error,
        InfinityEmbeddingError::Json { status_code: 500, message, source }
            if *message == source.to_string()
    ));
}

#[rstest]
#[case::array(b"[]")]
#[case::data_is_a_string(br#"{"data":"x"}"#)]
#[case::model_is_a_number(br#"{"model":3}"#)]
fn valid_json_of_the_wrong_shape_is_not_a_json_failure(#[case] raw: &[u8]) {
    assert!(matches!(
        decode_infinity_embedding(raw, 200),
        Err(InfinityEmbeddingError::Shape { .. })
    ));
}

#[test]
fn the_error_class_keeps_headers_and_the_synthetic_request() {
    let supplied = headers(&[("retry-after", "3")]);
    let error = get_error_class("busy", 429, &supplied);
    assert_eq!(error.to_string(), "Infinity answered with status 429: busy");
    assert!(matches!(
        &error,
        InfinityEmbeddingError::Provider { status_code: 429, message, headers }
            if message == "busy" && headers == &supplied
    ));
    assert_eq!(
        SYNTHETIC_REQUEST_URL,
        "https://github.com/michaelfeil/infinity"
    );
}

#[rstest]
fn replays_the_infinity_embedding_corpus(
    #[files("fuzz/corpus/infinity_embedding/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _reply = decode_infinity_embedding(&bytes, 502);
}
