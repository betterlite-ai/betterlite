#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use super::{
    EmbeddingDecodeError, SUPPORTED_OPENAI_PARAMS, decode_embedding_response, get_complete_url,
    map_openai_params, transform_embedding_request, transform_embedding_response,
    validate_environment,
};
use crate::embedding::EmbeddingInput;
use crate::openai_base_config::Headers;

fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

#[test]
fn validate_environment_sets_the_openrouter_headers() {
    let headers = validate_environment(Headers::new(), None, None, None);
    assert_eq!(
        headers,
        Headers::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://litellm.ai")
            ),
            (String::from("X-Title"), String::from("liteLLM")),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
        ])
    );
}

#[test]
fn validate_environment_reads_the_site_and_app_names() {
    let headers = validate_environment(
        Headers::new(),
        Some(&key("sk-or")),
        Some("https://example.com"),
        Some("my app"),
    );
    assert_eq!(
        headers.get("HTTP-Referer").map(String::as_str),
        Some("https://example.com")
    );
    assert_eq!(headers.get("X-Title").map(String::as_str), Some("my app"));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-or")
    );
}

#[test]
fn validate_environment_treats_an_empty_name_and_key_as_unset() {
    let headers = validate_environment(Headers::new(), Some(&key("")), Some(""), Some(""));
    assert_eq!(
        headers.get("HTTP-Referer").map(String::as_str),
        Some("https://litellm.ai")
    );
    assert_eq!(headers.get("X-Title").map(String::as_str), Some("liteLLM"));
    assert!(!headers.contains_key("Authorization"));
}

#[test]
fn validate_environment_overlays_the_caller_headers_case_sensitively() {
    let caller = Headers::from([(String::from("content-type"), String::from("text/plain"))]);
    let headers = validate_environment(caller, None, None, None);
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(headers.len(), 4);
}

#[rstest::rstest]
#[case::default_base(None, "https://openrouter.ai/api/v1/embeddings")]
#[case::empty_base(Some(""), "https://openrouter.ai/api/v1/embeddings")]
#[case::trailing_slashes(
    Some("https://proxy.example/v1///"),
    "https://proxy.example/v1/embeddings"
)]
fn get_complete_url_appends_the_endpoint(#[case] base: Option<&str>, #[case] expected: &str) {
    assert_eq!(get_complete_url(base), expected);
}

#[test]
fn transform_embedding_request_wraps_a_string_and_strips_one_prefix() {
    let input = EmbeddingInput::Text(String::from("hello"));
    let body = transform_embedding_request(
        "openrouter/google/gemini-embedding-001",
        &input,
        &Object::new(),
    );
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("google/gemini-embedding-001")))
    );
    assert_eq!(
        body.get("input"),
        Some(&Json::List(vec![Json::Text(String::from("hello"))]))
    );
}

#[test]
fn transform_embedding_request_strips_only_a_leading_prefix() {
    let input = EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]);
    let body = transform_embedding_request("x/openrouter/y", &input, &Object::new());
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("x/openrouter/y")))
    );
    assert_eq!(
        body.get("input"),
        Some(&Json::List(vec![
            Json::Text(String::from("a")),
            Json::Text(String::from("b"))
        ]))
    );
}

#[test]
fn transform_embedding_request_passes_token_inputs_as_numbers() {
    let input = EmbeddingInput::TokenLists(vec![vec![1, 2], vec![3]]);
    let body = transform_embedding_request("m", &input, &Object::new());
    let Json::List(lists) = body.get("input").unwrap() else {
        panic!("input is not a list");
    };
    assert_eq!(lists.len(), 2);
    assert_eq!(
        lists[0],
        Json::List(vec![
            Json::Number(serde_json::Number::from(1)),
            Json::Number(serde_json::Number::from(2))
        ])
    );
}

#[test]
fn transform_embedding_request_lets_an_option_win() {
    let input = EmbeddingInput::Text(String::from("hello"));
    let optional = Object::from([
        (String::from("model"), Json::Text(String::from("other"))),
        (
            String::from("input"),
            Json::List(vec![Json::Text(String::from("given"))]),
        ),
    ]);
    let body = transform_embedding_request("m", &input, &optional);
    assert_eq!(body.get("model"), Some(&Json::Text(String::from("other"))));
    assert_eq!(body.get("input"), Some(&optional["input"]));
}

#[test]
fn supported_params_are_the_four_embedding_names() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["timeout", "dimensions", "encoding_format", "user"]
    );
}

#[test]
fn map_openai_params_keeps_the_four_and_drops_the_rest() {
    let requested = Object::from([
        (
            String::from("dimensions"),
            Json::Number(serde_json::Number::from(8)),
        ),
        (String::from("user"), Json::Text(String::from("u"))),
        (
            String::from("temperature"),
            Json::Number(serde_json::Number::from(1)),
        ),
    ]);
    let mut optional = Object::new();
    map_openai_params(&requested, &mut optional);
    assert_eq!(optional.len(), 2);
    assert!(optional.contains_key("dimensions"));
    assert!(optional.contains_key("user"));
    assert!(!optional.contains_key("temperature"));
}

#[test]
fn decode_embedding_response_reads_the_standard_reply() {
    let raw = br#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"m","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
    let response = decode_embedding_response(raw).unwrap();
    assert_eq!(response.object, "list");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.usage.total_tokens, 2);
}

#[rstest::rstest]
#[case::invalid_json("{", true)]
#[case::empty("", true)]
#[case::array("[]", false)]
#[case::missing_usage(r#"{"object":"list","data":[],"model":"m"}"#, false)]
#[case::wrong_vector(
    r#"{"object":"list","data":[{"object":"embedding","embedding":"x","index":0}],"model":"m","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    false
)]
fn decode_embedding_response_reports_the_failure_kind(#[case] raw: &str, #[case] json: bool) {
    let error = decode_embedding_response(raw.as_bytes()).unwrap_err();
    if json {
        assert!(matches!(error, EmbeddingDecodeError::Json { .. }));
    } else {
        assert!(matches!(error, EmbeddingDecodeError::Shape { .. }));
    }
}

#[test]
fn transform_embedding_response_hands_the_text_to_the_post_call_hook() {
    let mut seen = Vec::new();
    let raw =
        br#"{"object":"list","data":[],"model":"m","usage":{"prompt_tokens":0,"total_tokens":0}}"#;
    let response =
        transform_embedding_response(raw, &mut |text| seen.push(text.to_owned())).unwrap();
    assert_eq!(seen, vec![std::str::from_utf8(raw).unwrap().to_owned()]);
    assert_eq!(response.model, "m");
}

#[test]
fn transform_embedding_response_decodes_an_invalid_body_after_the_hook() {
    let mut called = false;
    let error = transform_embedding_response(b"not json", &mut |_text| called = true).unwrap_err();
    assert!(called);
    assert!(matches!(error, EmbeddingDecodeError::Json { .. }));
}

#[rstest::rstest]
fn replays_the_openrouter_embedding_reply_corpus(
    #[files("fuzz/corpus/openrouter_embedding_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _reply = decode_embedding_response(&bytes);
}
