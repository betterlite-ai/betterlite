#![cfg(test)]

use betterlite_config::sections::embedding_host_settings::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::error::{JinaAiError, get_error_class};
use super::{
    BuiltinInput, DEFAULT_URL, SUPPORTED_OPENAI_PARAMS, get_complete_url, map_openai_params,
    resolve_credentials, transform_embedding_request, transform_embedding_response,
    validate_environment,
};
use crate::embedding::EmbeddingError;
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

fn settings(base: Option<&str>, key: Option<&str>) -> Settings {
    Settings {
        jina_ai_api_base: base.map(str::to_owned),
        jina_ai_api_key: key.map(secret),
        infinity_api_key: None,
    }
}

#[rstest]
#[case::explicit_wins(
    (Some("https://explicit"), Some("explicit")),
    (Some("https://env"), Some("env")),
    ("https://explicit", Some("explicit"))
)]
#[case::environment(
    (None, None),
    (Some("https://env"), Some("env")),
    ("https://env", Some("env"))
)]
#[case::empty_explicit_falls_through(
    (Some(""), Some("")),
    (Some("https://env"), Some("env")),
    ("https://env", Some("env"))
)]
#[case::unset((None, None), (None, None), ("https://api.jina.ai/v1", None))]
#[case::empty_explicit_unset((Some(""), Some("")), (None, None), ("https://api.jina.ai/v1", None))]
fn explicit_values_beat_the_environment_only_when_non_empty(
    #[case] explicit: (Option<&str>, Option<&str>),
    #[case] environment: (Option<&str>, Option<&str>),
    #[case] expected: (&str, Option<&str>),
) {
    let supplied = explicit.1.map(secret);
    let resolved = resolve_credentials(
        explicit.0,
        supplied.as_ref(),
        &settings(environment.0, environment.1),
    );
    let key = resolved
        .api_key
        .map(|secret| secret.expose_secret().to_owned());
    assert_eq!((resolved.api_base.as_str(), key.as_deref()), expected);
}

#[rstest]
#[case::appends(Some("https://jina.test/v1"), "https://jina.test/v1/embeddings")]
#[case::complete_endpoint_is_suffixed_again(
    Some("https://jina.test/v1/embeddings"),
    "https://jina.test/v1/embeddings/embeddings"
)]
#[case::trailing_slash_kept(Some("https://jina.test/"), "https://jina.test//embeddings")]
#[case::empty(Some(""), DEFAULT_URL)]
#[case::missing(None, DEFAULT_URL)]
fn complete_url_always_appends(#[case] base: Option<&str>, #[case] expected: &str) {
    assert_eq!(get_complete_url(base), expected);
}

#[rstest]
#[case::key(Some("secret"), &[], &[("Content-Type", "application/json"), ("Authorization", "Bearer secret")])]
#[case::no_key(None, &[], &[("Content-Type", "application/json")])]
#[case::empty_key(Some(""), &[], &[("Content-Type", "application/json")])]
#[case::caller_wins(Some("secret"), &[("Authorization", "Bearer mine"), ("Content-Type", "text/plain")], &[("Content-Type", "text/plain"), ("Authorization", "Bearer mine")])]
#[case::other_case_is_a_second_entry(None, &[("content-type", "text/plain")], &[("Content-Type", "application/json"), ("content-type", "text/plain")])]
fn headers_default_to_json_and_bearer(
    #[case] key: Option<&str>,
    #[case] supplied: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    let api_key = key.map(secret);
    assert_eq!(
        validate_environment(&headers(supplied), api_key.as_ref()),
        headers(expected)
    );
}

#[test]
fn only_dimensions_is_supported_and_mapped_unchanged() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["dimensions"]);
    let mut optional = object(r#"{"kept": 1}"#);
    let requested = object(r#"{"dimensions": 256, "encoding_format": "float", "user": "u"}"#);
    map_openai_params(&requested, &mut optional);
    assert_eq!(optional, object(r#"{"kept": 1, "dimensions": 256}"#));
    map_openai_params(&object(r#"{"user": "u"}"#), &mut optional);
    assert_eq!(optional, object(r#"{"kept": 1, "dimensions": 256}"#));
}

#[rstest]
#[case::scalar_is_wrapped(r#""hello""#, r#"["hello"]"#)]
#[case::text_list_is_kept(r#"["a", "b"]"#, r#"["a", "b"]"#)]
#[case::token_list_is_kept("[1, 2]", "[1, 2]")]
#[case::nested_tokens_are_kept("[[1], [2]]", "[[1], [2]]")]
#[case::scalar_number_is_wrapped("7", "[7]")]
#[case::nonstrings_stay_without_an_image(r#"["a", 3, null]"#, r#"["a", 3, null]"#)]
#[case::lone_image(r#""data:image/png;base64,aGVsbG8=""#, r#"[{"image": "aGVsbG8="}]"#)]
#[case::image_and_text(
    r#"["before", "data:image/png;base64,aGVsbG8=", "after"]"#,
    r#"[{"text": "before"}, {"image": "aGVsbG8="}, {"text": "after"}]"#
)]
#[case::nonstrings_are_dropped_after_an_image(
    r#"["data:image/png;base64,aGVsbG8=", 5, ["x"], null, "t"]"#,
    r#"[{"image": "aGVsbG8="}, {"text": "t"}]"#
)]
#[case::image_after_nonstring(
    r#"[5, "data:image/png;base64,aGVsbG8="]"#,
    r#"[{"image": "aGVsbG8="}]"#
)]
#[case::second_comma_component(
    r#"["data:image/png;base64,aGVsbG8=,tail"]"#,
    r#"[{"image": "aGVsbG8="}]"#
)]
#[case::invalid_payload_is_text(
    r#"["data:image/png;base64,***", 4]"#,
    r#"["data:image/png;base64,***", 4]"#
)]
#[case::missing_scheme_is_text(r#"["aGVsbG8=", 4]"#, r#"["aGVsbG8=", 4]"#)]
fn mixed_media_input_is_converted_only_after_a_data_uri(
    #[case] input: &str,
    #[case] expected: &str,
) {
    let body = transform_embedding_request("jina-clip-v2", &parse(input), &Object::new());
    assert_eq!(body.get("input"), Some(&parse(expected)));
    assert_eq!(body.get("model"), Some(&parse(r#""jina-clip-v2""#)));
}

#[test]
fn optional_input_loses_and_optional_model_wins() {
    let options = object(r#"{"input": "shadow", "model": "other", "dimensions": 64}"#);
    let body = transform_embedding_request("jina-clip-v2", &parse(r#"["real"]"#), &options);
    assert_eq!(
        body,
        object(r#"{"input": ["real"], "model": "other", "dimensions": 64}"#)
    );
}

#[test]
fn the_hook_sees_the_builtin_before_the_response_model_is_built() {
    let request = object(r#"{"model": "m", "input": ["a"]}"#);
    let key = secret("secret");
    let mut seen = Vec::new();
    let raw = br#"{"model":"jina","data":[{"object":"embedding","embedding":[1.5],"index":0}],"usage":{"prompt_tokens":2,"total_tokens":3}}"#;
    let response = transform_embedding_response(raw, Some(&key), &request, |call| {
        seen.push((
            call.input,
            call.api_key.map(|secret| secret.expose_secret().to_owned()),
            call.complete_input_dict.clone(),
            call.original_response.clone(),
        ));
    })
    .unwrap();
    assert_eq!(
        seen,
        vec![(
            BuiltinInput,
            Some("secret".to_owned()),
            request,
            serde_json::from_slice::<Value>(raw).unwrap(),
        )]
    );
    assert_eq!(response.model, "jina");
    assert_eq!(response.usage.total_tokens, 3);
}

#[rstest]
#[case::other_object(
    br#"{"object":"other","model":"jina","data":[],"usage":{"prompt_tokens":1,"total_tokens":1}}"#
)]
#[case::missing_object(
    br#"{"model":"jina","data":[],"usage":{"prompt_tokens":1,"total_tokens":1}}"#
)]
fn the_object_is_always_list(#[case] raw: &[u8]) {
    let response = transform_embedding_response(raw, None, &Object::new(), |_| {}).unwrap();
    assert_eq!(response.object, "list");
}

#[test]
fn the_hook_fires_even_when_the_response_model_cannot_be_built() {
    let mut calls = Vec::new();
    let result = transform_embedding_response(br#"{"data":"nope"}"#, None, &Object::new(), |_| {
        calls.push(());
    });
    assert!(matches!(result, Err(EmbeddingError::Decode { .. })));
    assert_eq!(calls.len(), 1);
}

#[rstest]
#[case::truncated(b"{")]
#[case::empty(b"")]
#[case::text(b"not json")]
fn invalid_json_keeps_the_decoding_error_and_skips_the_hook(#[case] raw: &[u8]) {
    let mut calls = Vec::new();
    let result = transform_embedding_response(raw, None, &Object::new(), |_| {
        calls.push(());
    });
    assert!(matches!(result, Err(EmbeddingError::Decode { .. })));
    assert!(calls.is_empty());
}

#[test]
fn the_error_class_carries_status_and_message_only() {
    let supplied = headers(&[("retry-after", "3")]);
    let error = get_error_class("slow down", 429, &supplied);
    assert_eq!(
        error,
        JinaAiError::Provider {
            status_code: 429,
            message: "slow down".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "Jina AI answered with status 429: slow down"
    );
}
