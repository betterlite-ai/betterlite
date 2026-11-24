#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_cache::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};
use betterlite_cache::types::embedding::{CachedEmbedding, EmbeddingPayload};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use pyo3::Python;
use serde::{Deserialize, Serialize};

use super::{cache_decode, cache_embedding, cache_envelope, cache_key};

const LOOKUP: &str =
    r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}}"#;

const WRITE: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}, "result": {"content": "hello"}}"#;

const WRITE_WITH_TTL: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "cache": {"ttl": 5}}, "result": {"content": "hello"}}"#;

const EMBEDDING_WRITE: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": ["a", "b"]}, "result": {"model": "text-embedding-3-small", "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}, {"object": "embedding", "embedding": [0.3, 0.4], "index": 1}], "usage": {"prompt_tokens": 7}}, "input": "b", "index": 1}"#;

const EMBEDDING_LOOKUP: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": "b"}}"#;

// A cached response older than the caller's s-maxage, with the bound a working caller would send.
const OLD_ENTRY_FRESH: &str = r#"{"kwargs": {"cache": {"s-maxage": 2000000000}}, "value": {"timestamp": 0, "response": {"content": "hello"}}}"#;

const OLD_ENTRY_EXPIRED: &str = r#"{"kwargs": {"cache": {"s-maxage": 60}}, "value": {"timestamp": 0, "response": {"content": "hello"}}}"#;

const OLD_ENTRY_S_MAX_AGE: &str = r#"{"kwargs": {"cache": {"s-max-age": 60}}, "value": {"timestamp": 0, "response": {"content": "hello"}}}"#;

const OLD_ENTRY_BOTH: &str = r#"{"kwargs": {"cache": {"s-maxage": 2000000000, "s-max-age": 60}}, "value": {"timestamp": 0, "response": {"content": "hello"}}}"#;

#[derive(Debug, Deserialize)]
struct Answer {
    cache_key: String,
    cached_data: CacheValue,
    ttl: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingAnswer {
    cache_key: String,
    cached_data: EmbeddingEnvelope,
    ttl: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingEnvelope {
    response: CachedEmbedding,
}

// A backend call answers the cached value a Python backend stored, under the call's own kwargs.
#[derive(Debug, Serialize)]
struct DecodeCall {
    kwargs: Json,
    value: CacheValue,
}

fn answer_ttl(settings: &str, request: &str) -> Option<f64> {
    let text = cache_envelope(settings, request).unwrap();
    serde_json::from_str::<Answer>(&text).unwrap().ttl
}

#[test]
fn cache_functions_reject_malformed_text() {
    Python::initialize();
    cache_key("not json", LOOKUP).unwrap_err();
    cache_key("{}", "not json").unwrap_err();
    cache_envelope("not json", WRITE).unwrap_err();
    cache_envelope("{}", "not json").unwrap_err();
    cache_decode("not json").unwrap_err();
    cache_embedding("not json", EMBEDDING_WRITE).unwrap_err();
    cache_embedding("{}", r#"{"kwargs": {"model": "text-embedding-3-small"}}"#).unwrap_err();
}

#[test]
fn cache_envelope_answers_the_key_cache_key_derives() {
    Python::initialize();
    let text = cache_envelope("{}", WRITE).unwrap();
    let answer: Answer = serde_json::from_str(&text).unwrap();
    assert_eq!(answer.cache_key, cache_key("{}", LOOKUP).unwrap());
    let CacheValue::Object(fields) = &answer.cached_data else {
        panic!("the cached data is an envelope object");
    };
    assert!(fields.contains_key(TIMESTAMP_FIELD));
    let response = CacheValue::Object(BTreeMap::from([(
        "content".to_owned(),
        CacheValue::Text("hello".to_owned()),
    )]));
    assert_eq!(fields.get(RESPONSE_FIELD), Some(&response));
}

#[test]
fn cache_key_honours_a_namespace_and_a_preset_key() {
    Python::initialize();
    let namespaced = cache_key(r#"{"namespace": "ns"}"#, LOOKUP).unwrap();
    assert!(namespaced.starts_with("ns:"));
    let preset = cache_key(
        "{}",
        r#"{"kwargs": {"model": "gpt-4o", "litellm_params": {"preset_cache_key": "given"}}}"#,
    )
    .unwrap();
    assert_eq!(preset, "given");
}

#[test]
fn cache_envelope_applies_the_ttl_rules() {
    Python::initialize();
    assert_eq!(answer_ttl("{}", WRITE), None);
    assert_eq!(answer_ttl(r#"{"ttl": 900}"#, WRITE), Some(900.0_f64));
    assert_eq!(
        answer_ttl(r#"{"ttl": 900, "default_in_memory_ttl": 600}"#, WRITE),
        Some(600.0_f64)
    );
    assert_eq!(
        answer_ttl(r#"{"ttl": 900, "default_in_redis_ttl": 600}"#, WRITE),
        Some(900.0_f64)
    );
    assert_eq!(
        answer_ttl(
            r#"{"type": "redis", "ttl": 900, "default_in_redis_ttl": 600}"#,
            WRITE
        ),
        Some(600.0_f64)
    );
    assert_eq!(answer_ttl(r#"{"ttl": 900}"#, WRITE_WITH_TTL), Some(5.0_f64));
}

#[test]
fn cache_decode_returns_the_stored_response() {
    Python::initialize();
    let text = cache_envelope("{}", WRITE).unwrap();
    let answer: Answer = serde_json::from_str(&text).unwrap();
    let call = DecodeCall {
        kwargs: serde_json::from_str("{}").unwrap(),
        value: answer.cached_data,
    };
    let request = serde_json::to_string(&call).unwrap();
    assert_eq!(cache_decode(&request).unwrap(), r#"{"content":"hello"}"#);
    assert_eq!(cache_decode(r#"{"kwargs": {}}"#).unwrap(), "null");
}

#[test]
fn cache_decode_expires_a_value_older_than_the_max_age() {
    Python::initialize();
    assert_eq!(
        cache_decode(OLD_ENTRY_FRESH).unwrap(),
        r#"{"content":"hello"}"#
    );
    assert_eq!(cache_decode(OLD_ENTRY_EXPIRED).unwrap(), "null");
    assert_eq!(cache_decode(OLD_ENTRY_S_MAX_AGE).unwrap(), "null");
    assert_eq!(
        cache_decode(OLD_ENTRY_BOTH).unwrap(),
        r#"{"content":"hello"}"#
    );
}

#[test]
fn cache_embedding_builds_the_entry_for_one_input() {
    Python::initialize();
    let text = cache_embedding("{}", EMBEDDING_WRITE).unwrap();
    let answer: EmbeddingAnswer = serde_json::from_str(&text).unwrap();
    assert_eq!(answer.cache_key, cache_key("{}", EMBEDDING_LOOKUP).unwrap());
    assert_eq!(answer.ttl, None);
    let record = answer.cached_data.response;
    assert_eq!(record.format_version, 2);
    assert_eq!(record.index, Some(1));
    assert_eq!(record.model.as_deref(), Some("text-embedding-3-small"));
    assert_eq!(record.prompt_tokens, Some(3));
    let Some(EmbeddingPayload::Vector(vector)) = record.embedding else {
        panic!("the cached embedding is a vector");
    };
    assert_eq!(vector.len(), 2);
    assert_eq!(vector[0].as_f64(), Some(0.3_f64));
}
