#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::error::PerplexityEmbeddingError;
use super::reply::{MAX_REPLY_BYTES, PerplexityEmbeddingReply, transform_embedding_response};
use super::{
    SUPPORTED_OPENAI_PARAMS, get_complete_url, get_error_class, map_openai_params,
    transform_embedding_request, validate_environment,
};
use crate::embedding::EmbeddingInput;
use crate::openai_base_config::Headers;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).unwrap())
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn decoded(body: &str) -> PerplexityEmbeddingReply {
    transform_embedding_response("pplx-embed-v1-0.6b", 200, body.as_bytes()).unwrap()
}

#[rstest]
#[case::no_base(None, "https://api.perplexity.ai/v1/embeddings")]
#[case::empty_base(Some(""), "https://api.perplexity.ai/v1/embeddings")]
#[case::plain_base(Some("https://mock.example"), "https://mock.example/v1/embeddings")]
#[case::base_with_v1(
    Some("https://mock.example/v1"),
    "https://mock.example/v1/v1/embeddings"
)]
#[case::trailing_slash(Some("https://mock.example/"), "https://mock.example//v1/embeddings")]
#[case::full_endpoint(
    Some("https://mock.example/x/embeddings"),
    "https://mock.example/x/embeddings"
)]
fn the_url_appends_the_full_suffix_without_trimming(
    #[case] api_base: Option<&str>,
    #[case] url: &str,
) {
    assert_eq!(get_complete_url(api_base), url);
}

#[test]
fn only_dimensions_and_encoding_format_are_mapped() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["dimensions", "encoding_format"]);
    let requested = object(r#"{"dimensions": 512, "encoding_format": "base64_int8", "user": "u"}"#);
    let mut optional = object(r#"{"kept": true}"#);
    map_openai_params(&requested, &mut optional);
    assert_eq!(
        optional,
        object(r#"{"kept": true, "dimensions": 512, "encoding_format": "base64_int8"}"#)
    );
}

#[rstest]
#[case::explicit_key(Some("sk-explicit"), &[("PERPLEXITYAI_API_KEY", "sk-env")], "Bearer sk-explicit")]
#[case::explicit_empty_key(Some(""), &[("PERPLEXITYAI_API_KEY", "sk-env")], "Bearer ")]
#[case::first_variable(None, &[("PERPLEXITYAI_API_KEY", "sk-first"), ("PERPLEXITY_API_KEY", "sk-second")], "Bearer sk-first")]
#[case::second_variable(None, &[("PERPLEXITY_API_KEY", "sk-second")], "Bearer sk-second")]
#[case::empty_first_variable(None, &[("PERPLEXITYAI_API_KEY", ""), ("PERPLEXITY_API_KEY", "sk-second")], "Bearer sk-second")]
#[case::no_key(None, &[], "Bearer None")]
fn the_headers_hold_only_the_bearer_and_the_json_type(
    #[case] api_key: Option<&str>,
    #[case] pairs: &[(&str, &str)],
    #[case] authorization: &str,
) {
    let key = api_key.map(|text| SecretString::from(text.to_owned()));
    let headers = validate_environment(key.as_ref(), &environment(pairs));
    assert_eq!(
        headers,
        Headers::from([
            (String::from("Authorization"), authorization.to_owned()),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
        ])
    );
}

#[test]
fn the_body_is_model_and_input_then_the_optional_params() {
    let input = EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]);
    let body =
        transform_embedding_request("pplx-embed-v1-4b", &input, &object(r#"{"dimensions": 8}"#))
            .unwrap();
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"model":"pplx-embed-v1-4b","input":["a","b"],"dimensions":8}"#
    );
}

#[test]
fn an_optional_model_or_input_replaces_the_request_value() {
    let input = EmbeddingInput::Text(String::from("hello"));
    let optional = object(r#"{"input": "override", "model": "other"}"#);
    let body = transform_embedding_request("pplx-embed-v1-4b", &input, &optional).unwrap();
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"model":"other","input":"override"}"#
    );
}

#[test]
fn token_input_stays_a_list_of_numbers() {
    let input = EmbeddingInput::TokenLists(vec![vec![1, 2], vec![3]]);
    let body = transform_embedding_request("m", &input, &Object::new()).unwrap();
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"model":"m","input":[[1,2],[3]]}"#
    );
}

#[test]
fn a_base64_string_decodes_to_signed_values_over_127() {
    let reply = decoded(
        r#"{"object":"list","model":"pplx-embed-v1-0.6b","data":[{"object":"embedding","index":0,"embedding":"AH+BgA=="}],"usage":{"prompt_tokens":3,"total_tokens":3}}"#,
    );
    let first = reply.data.first().unwrap();
    assert_eq!(
        first.get("embedding"),
        Some(&Json::List(vec![
            float(0.0_f64),
            float(1.0_f64),
            float(-1.0_f64),
            float(-1.007_874_015_748_031_5_f64),
        ]))
    );
    assert_eq!(first.get("index"), Some(&Json::Number(Number::from(0_u8))));
    assert_eq!(
        first.get("object"),
        Some(&Json::Text(String::from("embedding")))
    );
}

#[test]
fn a_list_and_any_other_value_stay_as_sent_and_extra_keys_stay() {
    let reply = decoded(
        r#"{"data":[{"embedding":[0.5, 2],"index":0,"extra":"kept"},{"embedding":7,"index":1},{"index":2}]}"#,
    );
    assert_eq!(
        reply.data,
        vec![
            object(r#"{"embedding":[0.5, 2],"index":0,"extra":"kept"}"#),
            object(r#"{"embedding":7,"index":1}"#),
            object(r#"{"index":2,"embedding":null}"#),
        ]
    );
}

#[test]
fn a_missing_model_and_object_take_the_request_model_and_list() {
    let reply = decoded("{}");
    assert_eq!(reply.model, "pplx-embed-v1-0.6b");
    assert_eq!(reply.object, "list");
    assert!(reply.data.is_empty());
    assert_eq!(reply.usage.prompt_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 0);
}

#[test]
fn the_reply_model_and_object_win_when_sent() {
    let reply = decoded(r#"{"model":"served","object":"other","data":null,"usage":null}"#);
    assert_eq!(reply.model, "served");
    assert_eq!(reply.object, "other");
    assert!(reply.data.is_empty());
}

#[rstest]
#[case::prompt_kept(r#"{"prompt_tokens":2,"total_tokens":5}"#, 2, 5)]
#[case::zero_prompt_falls_back(r#"{"prompt_tokens":0,"total_tokens":5}"#, 5, 5)]
#[case::missing_prompt_falls_back(r#"{"total_tokens":4}"#, 4, 4)]
#[case::missing_total(r#"{"prompt_tokens":3}"#, 3, 0)]
fn prompt_tokens_fall_back_to_total_tokens(
    #[case] usage: &str,
    #[case] prompt_tokens: u32,
    #[case] total_tokens: u32,
) {
    let reply = decoded(&format!(r#"{{"data":[],"usage":{usage}}}"#));
    assert_eq!(reply.usage.prompt_tokens, prompt_tokens);
    assert_eq!(reply.usage.total_tokens, total_tokens);
    assert_eq!(reply.usage.completion_tokens, 0);
}

#[test]
fn a_reply_that_is_not_json_keeps_the_status_and_the_text() {
    let error = transform_embedding_response("m", 502, b"upstream down").unwrap_err();
    assert_eq!(error.status_code(), 502);
    let PerplexityEmbeddingError::Provider { error: provider } = error else {
        panic!("a provider error");
    };
    assert_eq!(provider.message, "upstream down");
    assert_eq!(provider.request.method, "POST");
    assert_eq!(provider.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(provider.response.status_code, 502);
    assert_eq!(provider.headers, Some(Vec::new()));
}

#[rstest]
#[case::array("[]")]
#[case::number("3")]
#[case::item_not_object(r#"{"data":[1]}"#)]
#[case::usage_not_object(r#"{"usage":[1, 2]}"#)]
fn a_json_reply_of_the_wrong_shape_is_a_shape_error(#[case] body: &str) {
    let error = transform_embedding_response("m", 200, body.as_bytes()).unwrap_err();
    assert!(matches!(error, PerplexityEmbeddingError::Shape { .. }));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn a_malformed_base64_embedding_is_a_decode_error() {
    let body = r#"{"data":[{"embedding":"AH+B"},{"embedding":"not base64!"}]}"#;
    let error = transform_embedding_response("m", 200, body.as_bytes()).unwrap_err();
    assert!(matches!(error, PerplexityEmbeddingError::Base64 { .. }));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn a_reply_over_the_limit_is_refused_and_one_at_the_limit_is_read() {
    let over = vec![b' '; MAX_REPLY_BYTES.checked_add(1).unwrap()];
    let refused = transform_embedding_response("m", 200, &over).unwrap_err();
    assert!(matches!(refused, PerplexityEmbeddingError::TooLarge { .. }));
    assert_eq!(refused.status_code(), 500);
    let at_limit = vec![b' '; MAX_REPLY_BYTES];
    let read = transform_embedding_response("m", 200, &at_limit).unwrap_err();
    assert!(matches!(read, PerplexityEmbeddingError::Provider { .. }));
}

#[test]
fn a_reply_that_is_not_utf8_keeps_replacement_characters() {
    let PerplexityEmbeddingError::Provider { error } =
        transform_embedding_response("m", 502, b"\xffbad").unwrap_err()
    else {
        panic!("a provider error");
    };
    assert_eq!(error.message, "\u{fffd}bad");
}

#[test]
fn the_error_class_keeps_status_message_and_headers() {
    let headers = Headers::from([(String::from("x-request-id"), String::from("r1"))]);
    let error = get_error_class("rate limited", 429, &headers);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rate limited");
    assert_eq!(
        error.headers,
        Some(vec![(String::from("x-request-id"), String::from("r1"))])
    );
    assert_eq!(error.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(error.response.status_code, 429);
}

#[test]
fn an_encode_error_answers_500() {
    let source = serde_json::from_str::<u8>("x").unwrap_err();
    assert_eq!(
        PerplexityEmbeddingError::Encode { source }.status_code(),
        500
    );
}
