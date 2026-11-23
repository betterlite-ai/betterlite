#![cfg(test)]

use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::embedding::EmbeddingPayload;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redact::structured::Payload;
use serde::Deserialize;

use super::{
    CacheCall, embedding_text, envelope_text, flush_size, hooks, parse_call, parse_settings,
    request_ttl, semantic_text, value_text,
};
use crate::cache::error::CacheError;

const CALL: &str = r#"{"kwargs": {"model": "gpt-4o", "cache_key": "preset", "cache": {"ttl": 30, "s-maxage": 5, "namespace": "ns"}, "metadata": {"redis_namespace": "other"}}}"#;

#[derive(Debug, Deserialize)]
struct PreparedAnswer {
    cache_key: String,
    cached_data: PreparedData,
    ttl: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PreparedData {
    timestamp: CacheValue,
    response: CacheValue,
}

fn call(text: &str) -> CacheCall {
    parse_call(text).unwrap()
}

#[test]
fn cache_request_kwargs_become_an_envelope_request() {
    let request = call(CALL).envelope_request().unwrap();
    assert_eq!(request.cache_key.as_deref(), Some("preset"));
    assert_eq!(request.control.ttl, Some(30_i64));
    assert_eq!(request.control.s_maxage, Some(5_i64));
    assert_eq!(request.control.namespace.as_deref(), Some("ns"));
    assert!(request.key.params.iter().any(|param| param.name == "model"));
    assert!(!request.key.params.iter().any(|param| param.name == "cache"));
    assert_eq!(
        request
            .key
            .metadata
            .and_then(|fields| fields.get("redis_namespace").map(|value| value.render())),
        Some("other".to_owned())
    );
}

#[test]
fn cache_request_without_a_cache_key_derives_none() {
    let request = call(r#"{"kwargs": {"model": "gpt-4o"}}"#)
        .envelope_request()
        .unwrap();
    assert_eq!(request.cache_key, None);
    assert_eq!(request.control.ttl, None);
}

#[test]
fn cache_request_rejects_an_unknown_key_or_a_malformed_text() {
    assert!(matches!(
        parse_call(r#"{"kwargs": {}, "unknown": 1}"#),
        Err(CacheError::Request { .. })
    ));
    assert!(matches!(
        parse_call("not json"),
        Err(CacheError::Request { .. })
    ));
    assert!(matches!(
        call(r#"{"kwargs": {"cache": {"ttl": "soon"}}}"#).envelope_request(),
        Err(CacheError::Handler { .. })
    ));
}

#[test]
fn cache_request_index_defaults_to_zero() {
    assert_eq!(call(r#"{"kwargs": {}}"#).result_index(), 0);
    assert_eq!(call(r#"{"kwargs": {}, "index": 3}"#).result_index(), 3);
}

#[test]
fn cache_request_a_missing_result_or_input_is_an_error() {
    assert!(matches!(
        call(r#"{"kwargs": {}}"#).result_value(),
        Err(CacheError::MissingResult)
    ));
    assert!(matches!(
        call(r#"{"kwargs": {}, "result": "x"}"#).embedding_result(),
        Err(CacheError::Result { .. })
    ));
    assert!(matches!(
        call(r#"{"kwargs": {}}"#).input_value(),
        Err(CacheError::MissingInput)
    ));
    assert_eq!(
        call(r#"{"kwargs": {}, "input": "hello"}"#)
            .input_value()
            .unwrap()
            .render(),
        "hello"
    );
}

#[test]
fn cache_request_settings_default_the_type_and_reject_an_unknown_key() {
    let settings = parse_settings("{}").unwrap();
    assert!(matches!(settings.cache_type, LiteLlmCacheType::Local));
    assert!(matches!(
        parse_settings(r#"{"type": "local", "unknown": 1}"#),
        Err(CacheError::Settings { .. })
    ));
    assert!(matches!(
        parse_settings("not json"),
        Err(CacheError::Settings { .. })
    ));
}

#[test]
fn cache_result_text_becomes_a_cache_value() {
    let result = call(r#"{"kwargs": {}, "result": {"content": "hi"}}"#)
        .result_value()
        .unwrap();
    assert_eq!(
        result,
        CacheValue::Object(
            [("content".to_owned(), CacheValue::Text("hi".to_owned()))]
                .into_iter()
                .collect()
        )
    );
}

#[test]
fn cache_result_envelope_holds_the_timestamp_and_the_response() {
    let text = envelope_text(
        "key",
        12.5_f64,
        &CacheValue::Text("hi".to_owned()),
        Some(30.0_f64),
    )
    .unwrap();
    let answer: PreparedAnswer = serde_json::from_str(&text).unwrap();
    assert_eq!(answer.cache_key, "key");
    assert_eq!(answer.cached_data.timestamp, CacheValue::Float(12.5_f64));
    assert_eq!(
        answer.cached_data.response,
        CacheValue::Text("hi".to_owned())
    );
    assert_eq!(answer.ttl, Some(30.0_f64));
}

#[test]
fn cache_result_a_null_cached_value_is_a_miss() {
    assert_eq!(
        call(r#"{"kwargs": {}, "value": null}"#)
            .cached_value()
            .unwrap(),
        None
    );
    assert_eq!(call(r#"{"kwargs": {}}"#).cached_value().unwrap(), None);
    assert_eq!(
        call(r#"{"kwargs": {}, "value": "text"}"#)
            .cached_value()
            .unwrap(),
        Some(CacheValue::Text("text".to_owned()))
    );
    assert_eq!(value_text(&None).unwrap(), "null");
    assert_eq!(value_text(&Some(CacheValue::Integer(7_i64))).unwrap(), "7");
}

#[test]
fn cache_result_semantic_text_carries_the_response_and_the_similarity() {
    let text = semantic_text(&Some(CacheValue::Text("hi".to_owned())), None).unwrap();
    let answer: (Option<CacheValue>, Option<f64>) = serde_json::from_str(&text).unwrap();
    assert_eq!(answer.0, Some(CacheValue::Text("hi".to_owned())));
    assert_eq!(answer.1, None);
}

#[test]
fn cache_result_the_cache_dict_ttl_wins_over_the_cache_ttl() {
    let request = call(CALL).envelope_request().unwrap();
    assert_eq!(request_ttl(&request, Some(600.0_f64)), Some(30.0_f64));
    let plain = call(r#"{"kwargs": {}}"#).envelope_request().unwrap();
    assert_eq!(request_ttl(&plain, Some(600.0_f64)), Some(600.0_f64));
}

#[test]
fn cache_result_the_embedding_response_keeps_its_records() {
    let result = call(
        r#"{"kwargs": {}, "result": {"model": "text-embedding-3", "data": [{"embedding": [0.5, 0.25], "index": 0, "object": "embedding"}], "usage": {"prompt_tokens": 2}}}"#,
    )
    .embedding_result()
    .unwrap();
    assert_eq!(result.model.as_deref(), Some("text-embedding-3"));
    assert_eq!(result.data.len(), 1);
    assert_eq!(result.data[0].index, Some(0_i64));
    assert!(matches!(
        result.data[0].embedding,
        Some(EmbeddingPayload::Vector(ref numbers)) if numbers.len() == 2
    ));
}

#[test]
fn cache_result_the_embedding_envelope_keeps_the_cached_data() {
    let mut cached_data = std::collections::BTreeMap::new();
    cached_data.insert("response".to_owned(), CacheValue::Text("cached".to_owned()));
    let entry = betterlite_cache::embedding::embedding_cache::EmbeddingCacheEntry {
        cache_key: "embedding-key".to_owned(),
        cached_data: CacheValue::Object(cached_data),
        cached_embedding: CacheValue::Null,
        ttl: Some(45.0_f64),
    };
    let text = embedding_text(&entry).unwrap();
    let answer: PreparedAnswer = serde_json::from_str(&text).unwrap();
    assert_eq!(answer.cache_key, "embedding-key");
    assert_eq!(
        answer.cached_data.response,
        CacheValue::Text("cached".to_owned())
    );
    assert_eq!(answer.ttl, Some(45.0_f64));
}

#[test]
fn cache_result_the_flush_size_must_fit_an_i64() {
    assert_eq!(flush_size(None).unwrap(), None);
    assert_eq!(flush_size(Some(1000_usize)).unwrap(), Some(1000_i64));
    assert!(matches!(
        flush_size(Some(usize::MAX)),
        Err(CacheError::FlushSize { .. })
    ));
}

#[test]
fn cache_result_the_hooks_keep_set_verbose_off() {
    let built = hooks().unwrap();
    assert_eq!(built.set_verbose, Payload::Bool(false));
}
