#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::reply::{EmbeddingReply, EmbeddingUsage, decode_embedding_reply};
use super::request::{complete_url, transform_embedding_request, validate_environment};
use crate::openai_base_config::Headers;
use crate::ovhcloud_chat::chunk::chunk_parser;

fn object(value: serde_json::Value) -> Object {
    serde_json::from_value(value).expect("a JSON object")
}

fn key(text: &str) -> SecretString {
    SecretString::from(text)
}

fn default_headers(authorization: &str) -> Headers {
    Headers::from([
        (String::from("Authorization"), String::from(authorization)),
        (String::from("accept"), String::from("application/json")),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ])
}

#[rstest]
#[case::default_base(None, "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/embeddings")]
#[case::given_base(
    Some("https://proxy.example/v1"),
    "https://proxy.example/v1/embeddings"
)]
#[case::trailing_slashes_are_stripped(
    Some("https://proxy.example/v1//"),
    "https://proxy.example/v1/embeddings"
)]
#[case::empty_base_stays_empty(Some(""), "/embeddings")]
fn complete_url_uses_the_default_base_or_the_given_one(
    #[case] api_base: Option<&str>,
    #[case] url: &str,
) {
    assert_eq!(complete_url(api_base), url);
}

#[rstest]
#[case::no_key_at_all(None, None, "Bearer None")]
#[case::explicit_key(Some("explicit"), None, "Bearer explicit")]
#[case::env_key(None, Some("env"), "Bearer env")]
#[case::explicit_key_wins(Some("explicit"), Some("env"), "Bearer explicit")]
#[case::an_empty_explicit_key_stays_empty(Some(""), Some("env"), "Bearer ")]
#[case::an_empty_env_key_stays_empty(None, Some(""), "Bearer ")]
fn validate_environment_writes_the_bearer_default(
    #[case] api_key: Option<&str>,
    #[case] env_api_key: Option<&str>,
    #[case] authorization: &str,
) {
    let explicit = api_key.map(key);
    let from_env = env_api_key.map(key);
    let headers = validate_environment(&Headers::new(), explicit.as_ref(), from_env.as_ref());
    assert_eq!(headers, default_headers(authorization));
}

#[test]
fn validate_environment_lets_the_caller_headers_win() {
    let caller = Headers::from([
        (String::from("Authorization"), String::from("Bearer caller")),
        (String::from("Content-Type"), String::from("text/plain")),
        (String::from("x-extra"), String::from("1")),
    ]);
    let headers = validate_environment(&caller, Some(&key("explicit")), None);
    assert_eq!(
        headers,
        Headers::from([
            (String::from("Authorization"), String::from("Bearer caller")),
            (String::from("Content-Type"), String::from("text/plain")),
            (String::from("accept"), String::from("application/json")),
            (String::from("x-extra"), String::from("1")),
        ])
    );
}

#[test]
fn a_lowercase_authorization_header_does_not_replace_the_default_one() {
    let caller = Headers::from([(String::from("authorization"), String::from("Bearer caller"))]);
    let headers = validate_environment(&caller, None, None);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer None")
    );
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn transform_embedding_request_puts_input_and_model_first() {
    let optional = object(json!({"user": "u", "dimensions": 3}));
    let body = transform_embedding_request("m", &Json::Text(String::from("hi")), &optional);
    assert_eq!(
        body.keys().cloned().collect::<Vec<String>>(),
        vec!["input", "model", "user", "dimensions"]
    );
    assert_eq!(
        serde_json::to_value(&body).expect("a body"),
        json!({"input": "hi", "model": "m", "user": "u", "dimensions": 3})
    );
}

#[test]
fn an_option_named_input_or_model_wins() {
    let optional = object(json!({"model": "over", "input": "over"}));
    let body = transform_embedding_request("m", &Json::Text(String::from("hi")), &optional);
    assert_eq!(
        serde_json::to_value(&body).expect("a body"),
        json!({"input": "over", "model": "over"})
    );
}

#[test]
fn decode_embedding_reply_copies_model_data_and_object() {
    let reply = decode_embedding_reply(
        br#"{"model":"up","data":[{"object":"embedding","embedding":[1,2],"index":0}],"object":"list","usage":{"prompt_tokens":3,"total_tokens":4}}"#,
        200,
        &Headers::new(),
    )
    .expect("a reply");
    assert_eq!(
        reply,
        EmbeddingReply {
            model: Some(Json::Text(String::from("up"))),
            data: Some(Json::List(vec![Json::Object(object(
                json!({"object": "embedding", "embedding": [1, 2], "index": 0})
            ))])),
            object: Some(Json::Text(String::from("list"))),
            usage: EmbeddingUsage {
                prompt_tokens: 3,
                total_tokens: 4,
            },
        }
    );
}

#[rstest]
#[case::null_fields_are_null(json!({"model": null, "data": null, "object": null}), 0, 0)]
#[case::absent_fields_are_null(json!({}), 0, 0)]
#[case::each_counter_defaults_alone(json!({"usage": {"prompt_tokens": 3}}), 3, 0)]
#[case::the_total_counter_alone(json!({"usage": {"total_tokens": 4}}), 0, 4)]
#[case::a_null_usage_contributes_zero(json!({"usage": null}), 0, 0)]
fn decode_embedding_reply_defaults_absent_parts(
    #[case] source: serde_json::Value,
    #[case] prompt_tokens: i64,
    #[case] total_tokens: i64,
) {
    let body = serde_json::to_vec(&source).expect("bytes");
    let reply = decode_embedding_reply(&body, 200, &Headers::new()).expect("a reply");
    let encoded = json!({
        "model": reply.model,
        "data": reply.data,
        "object": reply.object,
        "usage": {
            "prompt_tokens": reply.usage.prompt_tokens,
            "total_tokens": reply.usage.total_tokens,
        },
    });
    let expected = json!({
        "model": source.get("model").cloned().unwrap_or(json!(null)),
        "data": source.get("data").cloned().unwrap_or(json!(null)),
        "object": source.get("object").cloned().unwrap_or(json!(null)),
        "usage": {"prompt_tokens": prompt_tokens, "total_tokens": total_tokens},
    });
    assert_eq!(encoded, expected);
}

#[test]
fn decode_embedding_reply_reports_a_body_that_is_not_json() {
    let headers = Headers::from([(String::from("x-request-id"), String::from("req-1"))]);
    let error = decode_embedding_reply(b"not json", 502, &headers).expect_err("an error");
    assert_eq!(error.message, "not json");
    assert_eq!(error.status_code, Some(502));
    assert_eq!(error.headers, headers);
}

#[test]
fn decode_embedding_reply_reports_a_json_body_that_is_not_an_object() {
    let error = decode_embedding_reply(b"[1,2]", 200, &Headers::new()).expect_err("an error");
    assert_eq!(error.message, "[1,2]");
    assert_eq!(error.status_code, Some(200));
}

#[rstest]
fn replays_the_ovhcloud_reply_corpus(#[files("fuzz/corpus/ovhcloud_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus file");
    let _embedding = decode_embedding_reply(&bytes, 200, &Headers::new());
    let _chunk = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| serde_json::from_value::<Object>(value).ok())
        .map(|chunk| chunk_parser(&chunk));
}
