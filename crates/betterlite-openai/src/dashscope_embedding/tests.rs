#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::error::get_error_class;
use super::identity::{FamilyIdentity, FamilyKeyError, resolve_key};
use super::reply::decode_dashscope_embedding;
use super::{
    DASHSCOPE_DEFAULT_BASE, DASHSCOPE_MISSING_KEY_MESSAGE, DashscopeEmbeddingConfig,
    map_openai_params, transform_embedding_request,
};
use crate::embedding::EmbeddingVector;
use crate::openai_base_config::Headers;

fn plain_identity(env_key: Option<&str>, env_base: Option<&str>) -> FamilyIdentity {
    FamilyIdentity {
        missing_key_message: DASHSCOPE_MISSING_KEY_MESSAGE.to_owned(),
        default_base: DASHSCOPE_DEFAULT_BASE.to_owned(),
        env_base: env_base.map(str::to_owned),
        env_key: env_key.map(str::to_owned),
        fallback_key: None,
        empty_key_is_set: true,
    }
}

fn config(env_key: Option<&str>, env_base: Option<&str>) -> DashscopeEmbeddingConfig {
    DashscopeEmbeddingConfig {
        identity: plain_identity(env_key, env_base),
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn params(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[test]
fn resolve_key_prefers_the_argument_over_the_environment() {
    let identity = plain_identity(Some("sk-env"), None);
    assert_eq!(resolve_key(&identity, Some("sk-arg")).unwrap(), "sk-arg");
    assert_eq!(resolve_key(&identity, None).unwrap(), "sk-env");
}

#[test]
fn resolve_key_takes_an_empty_argument_for_a_plain_dashscope_brand() {
    let identity = plain_identity(Some("sk-env"), None);
    assert_eq!(resolve_key(&identity, Some("")).unwrap(), "");
}

#[test]
fn resolve_key_skips_an_empty_argument_for_a_brand_key() {
    let identity = FamilyIdentity {
        empty_key_is_set: false,
        fallback_key: Some("sk-fallback".to_owned()),
        ..plain_identity(Some("sk-env"), None)
    };
    assert_eq!(resolve_key(&identity, Some("")).unwrap(), "sk-env");
    assert_eq!(resolve_key(&identity, Some("sk-arg")).unwrap(), "sk-arg");
}

#[test]
fn resolve_key_falls_through_to_the_fallback_and_then_the_message() {
    let identity = FamilyIdentity {
        empty_key_is_set: false,
        env_key: Some(String::new()),
        fallback_key: Some("sk-fallback".to_owned()),
        ..plain_identity(None, None)
    };
    assert_eq!(resolve_key(&identity, None).unwrap(), "sk-fallback");
    let identity = FamilyIdentity {
        empty_key_is_set: false,
        fallback_key: None,
        ..plain_identity(None, None)
    };
    let error = resolve_key(&identity, None).unwrap_err();
    assert_eq!(
        error,
        FamilyKeyError::Missing {
            message: DASHSCOPE_MISSING_KEY_MESSAGE.to_owned()
        }
    );
}

#[rstest]
#[case::only_the_supported_keys(&[("dimensions", Json::Number(Number::from(256))), ("rank", text("x"))], &["dimensions"])]
#[case::a_null_value_is_skipped(&[("user", Json::Null)], &[])]
#[case::an_unknown_key_is_dropped(&[("encoding_format", text("float")), ("foo", text("bar"))], &["encoding_format"])]
fn map_openai_params_keeps_only_the_supported_non_null_keys(
    #[case] entries: &[(&str, Json)],
    #[case] expected: &[&str],
) {
    let mapped = map_openai_params(&params(entries), IndexMap::new());
    let kept: Vec<&str> = mapped.keys().map(String::as_str).collect();
    assert_eq!(kept, expected);
}

#[test]
fn map_openai_params_overwrites_an_existing_supported_value() {
    let mapped = map_openai_params(
        &params(&[("user", text("new"))]),
        params(&[
            ("user", text("old")),
            ("top_k", Json::Number(Number::from(3))),
        ]),
    );
    assert_eq!(mapped.get("user"), Some(&text("new")));
    assert_eq!(mapped.get("top_k"), Some(&Json::Number(Number::from(3))));
}

#[test]
fn headers_carry_the_bearer_key_and_the_caller_overlay() {
    let config = config(None, None);
    let mut caller = Headers::new();
    caller.insert("x-custom".to_owned(), "value".to_owned());
    caller.insert("Content-Type".to_owned(), "application/custom".to_owned());
    let headers = config
        .validate_environment(Some("sk-arg"), &caller)
        .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-arg")
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/custom")
    );
    assert_eq!(headers.get("x-custom").map(String::as_str), Some("value"));
}

#[test]
fn headers_use_the_environment_key_when_no_argument_is_given() {
    let config = config(Some("sk-env"), None);
    let headers = config.validate_environment(None, &Headers::new()).unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-env")
    );
    assert_eq!(
        headers.get("Accept"),
        None,
        "the embedding headers carry Content-Type and Authorization only"
    );
}

#[test]
fn headers_fail_with_the_missing_key_message() {
    let config = config(None, None);
    let error = config
        .validate_environment(None, &Headers::new())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "DashScope API key is required. Set 'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
    );
}

#[rstest]
#[case::argument_base(
    None,
    Some("https://example.test/v1"),
    "https://example.test/v1/embeddings"
)]
#[case::environment_base(Some("https://env.test/v1"), None, "https://env.test/v1/embeddings")]
#[case::default_base(
    None,
    None,
    "https://dashscope.aliyuncs.com/compatible-mode/v1/embeddings"
)]
#[case::trailing_slashes(
    Some("https://example.test/v1///"),
    None,
    "https://example.test/v1/embeddings"
)]
#[case::already_an_embeddings_base(
    Some("https://example.test/v1/embeddings"),
    None,
    "https://example.test/v1/embeddings"
)]
#[case::an_empty_argument_base(None, Some(""), "/embeddings")]
fn complete_url_resolves_the_base_and_appends_the_route(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(config(None, env_base).complete_url(api_base), expected);
}

#[test]
fn the_request_body_carries_the_options_that_are_set() {
    let body = transform_embedding_request(
        "text-embedding-v4",
        &Json::List(vec![text("hello")]),
        &params(&[
            ("dimensions", Json::Number(Number::from(64))),
            ("user", text("max")),
            ("encoding_format", Json::Null),
        ]),
    );
    assert_eq!(
        body.get("model"),
        Some(&Json::Text("text-embedding-v4".to_owned()))
    );
    assert_eq!(body.get("input"), Some(&Json::List(vec![text("hello")])));
    assert_eq!(
        body.get("dimensions"),
        Some(&Json::Number(Number::from(64)))
    );
    assert_eq!(body.get("user"), Some(&Json::Text("max".to_owned())));
    assert_eq!(body.get("encoding_format"), None);
}

#[test]
fn the_reply_decodes_the_data_model_and_usage() {
    let body = br#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.5,-1]}],"model":"text-embedding-v4","usage":{"prompt_tokens":9,"total_tokens":11},"id":"emb-1"}"#;
    let reply = decode_dashscope_embedding(body, 200, "requested-model").unwrap();
    assert_eq!(reply.id.as_deref(), Some("emb-1"));
    assert_eq!(reply.response.object, "list");
    assert_eq!(reply.response.model, "text-embedding-v4");
    assert_eq!(reply.response.usage.prompt_tokens, 9);
    assert_eq!(reply.response.usage.completion_tokens, 0);
    assert_eq!(reply.response.usage.total_tokens, 11);
    let record = &reply.response.data[0];
    assert_eq!(record.object, "embedding");
    assert_eq!(record.index, 0);
    match &record.embedding {
        EmbeddingVector::Floats(values) => assert_eq!(values.len(), 2),
        EmbeddingVector::Base64(_) => panic!("DashScope answers floats here"),
    }
}

#[test]
fn the_reply_defaults_the_missing_parts() {
    let reply = decode_dashscope_embedding(br#"{}"#, 200, "requested-model").unwrap();
    assert_eq!(reply.id, None);
    assert!(reply.response.data.is_empty());
    assert_eq!(reply.response.model, "requested-model");
    assert_eq!(reply.response.usage.prompt_tokens, 0);
    assert_eq!(reply.response.usage.total_tokens, 0);
}

#[test]
fn the_reply_defaults_the_total_to_the_prompt_tokens() {
    let reply =
        decode_dashscope_embedding(br#"{"usage":{"prompt_tokens":4}}"#, 200, "requested-model")
            .unwrap();
    assert_eq!(reply.response.usage.prompt_tokens, 4);
    assert_eq!(reply.response.usage.total_tokens, 4);
}

#[test]
fn the_reply_keeps_a_base64_embedding_as_text() {
    let body = br#"{"data":[{"object":"embedding","index":3,"embedding":"AAAA"}]}"#;
    let reply = decode_dashscope_embedding(body, 200, "model").unwrap();
    assert_eq!(reply.response.data[0].index, 3);
    match &reply.response.data[0].embedding {
        EmbeddingVector::Base64(text) => assert_eq!(text, "AAAA"),
        EmbeddingVector::Floats(_) => panic!("a text embedding stays base64"),
    }
}

#[test]
fn the_reply_reads_a_null_token_count_as_unset() {
    let body = br#"{"usage":{"prompt_tokens":null,"total_tokens":5}}"#;
    let reply = decode_dashscope_embedding(body, 200, "model").unwrap();
    assert_eq!(reply.response.usage.prompt_tokens, 0);
    assert_eq!(reply.response.usage.total_tokens, 5);
}

#[test]
fn the_reply_reports_a_non_json_body_with_the_reply_status() {
    let error = decode_dashscope_embedding(b"not json", 502, "model").unwrap_err();
    assert_eq!(error.status_code, 502);
    assert!(
        error
            .message
            .starts_with("Failed to parse DashScope response as JSON: ")
    );
    assert!(error.headers.is_empty());
}

#[rstest]
#[case::an_object_error(
    br#"{"error":{"message":"invalid api key","code":"InvalidApiKey"}}"#,
    "invalid api key"
)]
#[case::a_text_error(br#"{"error":"rate limited"}"#, "rate limited")]
#[case::an_object_without_a_message(br#"{"error":{"code":"x"}}"#, r#"{"code":"x"}"#)]
fn the_reply_reports_an_error_key_as_a_dashscope_error(
    #[case] body: &[u8],
    #[case] expected: &str,
) {
    let error = decode_dashscope_embedding(body, 400, "model").unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, expected);
}

#[rstest]
#[case::a_reply_that_is_not_an_object(b"[]")]
#[case::index_that_is_not_a_number(
    br#"{"data":[{"object":"embedding","index":"0","embedding":[]}]}"#
)]
#[case::an_embedding_that_is_not_a_list(
    br#"{"data":[{"object":"embedding","index":0,"embedding":{}}]}"#
)]
#[case::usage_that_is_not_an_object(br#"{"usage":[]}"#)]
#[case::a_token_count_that_is_negative(br#"{"usage":{"prompt_tokens":-1}}"#)]
#[case::an_id_that_is_not_text(br#"{"id":7}"#)]
fn the_reply_rejects_a_shape_the_records_cannot_hold(#[case] body: &[u8]) {
    let error = decode_dashscope_embedding(body, 200, "model").unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("wrong type"));
}

#[test]
fn the_error_class_carries_the_status_message_and_headers() {
    let mut headers = Headers::new();
    headers.insert("x-request-id".to_owned(), "req-1".to_owned());
    let error = get_error_class("bad request", 400, Some(&headers));
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "bad request");
    assert_eq!(
        error.headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    let bare = get_error_class("bad request", 400, None);
    assert!(bare.headers.is_empty());
}

#[rstest]
fn replays_the_dashscope_embedding_reply_corpus(
    #[files("fuzz/corpus/dashscope_embedding_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("read corpus file");
    let _decoded = decode_dashscope_embedding(&bytes, 200, "text-embedding-v4");
    let _failed = decode_dashscope_embedding(&bytes, 500, "text-embedding-v4");
}
