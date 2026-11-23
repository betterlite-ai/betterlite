#![cfg(test)]

use pyo3::prelude::*;
use serde::Deserialize;

use super::NativeCache;
use crate::cache::register;

const LOOKUP: &str =
    r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}}"#;

const WRITE: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}, "result": {"content": "hello"}}"#;

const EMBEDDING_WRITE: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": ["a", "b"]}, "result": {"model": "text-embedding-3-small", "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}, {"object": "embedding", "embedding": [0.3, 0.4], "index": 1}], "usage": {"prompt_tokens": 7}}, "input": "b", "index": 1}"#;

const EMBEDDING_LOOKUP_A: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": "a"}}"#;

const EMBEDDING_LOOKUP_B: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": "b"}}"#;

#[derive(Debug, Deserialize)]
struct EntryAnswer {
    cache_key: String,
}

#[test]
fn register_adds_the_cache_class_and_functions() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeCache").unwrap());
        assert!(module.hasattr("cache_key").unwrap());
        assert!(module.hasattr("cache_envelope").unwrap());
        assert!(module.hasattr("cache_decode").unwrap());
        assert!(module.hasattr("cache_embedding").unwrap());
    });
}

#[test]
fn a_malformed_settings_text_is_an_error_not_a_panic() {
    Python::initialize();
    assert!(NativeCache::new("not json").is_err());
    assert!(NativeCache::new(r#"{"unknown": 1}"#).is_err());
    assert!(NativeCache::new(r#"{"type": "redis"}"#).is_err());
}

#[test]
fn the_cache_round_trips_a_written_value() {
    Python::initialize();
    let cache = NativeCache::new("{}").unwrap();
    assert_eq!(cache.lookup(LOOKUP).unwrap(), "null");
    cache.write(WRITE).unwrap();
    assert_eq!(cache.lookup(LOOKUP).unwrap(), r#"{"content":"hello"}"#);
    assert_eq!(cache.cache_key(LOOKUP).unwrap().len(), 64);
    assert!(cache.write(r#"{"kwargs": {"model": "gpt-4o"}}"#).is_err());
}

#[test]
fn a_local_store_closes_without_error() {
    Python::initialize();
    let cache = NativeCache::new("{}").unwrap();
    cache.write(WRITE).unwrap();
    Python::attach(|py| cache.close(py)).unwrap();
}

#[test]
fn the_semantic_lookup_answers_the_response_and_no_similarity() {
    Python::initialize();
    let cache = NativeCache::new("{}").unwrap();
    cache.write(WRITE).unwrap();
    assert_eq!(
        cache.lookup_semantic(LOOKUP).unwrap(),
        r#"[{"content":"hello"},null]"#
    );
}

#[test]
fn the_embedding_entry_names_the_input_it_was_built_for() {
    Python::initialize();
    let cache = NativeCache::new("{}").unwrap();
    let text = cache.write_embedding(EMBEDDING_WRITE).unwrap();
    let entry: EntryAnswer = serde_json::from_str(&text).unwrap();
    assert_eq!(
        entry.cache_key,
        cache.cache_key(EMBEDDING_LOOKUP_B).unwrap()
    );
    assert_eq!(cache.lookup(EMBEDDING_LOOKUP_B).unwrap(), "null");
}

#[test]
fn the_pipeline_writes_every_input_and_a_batch_write_is_not_implemented() {
    Python::initialize();
    let cache = NativeCache::new("{}").unwrap();
    Python::attach(|py| cache.write_pipeline(py, EMBEDDING_WRITE)).unwrap();
    assert_ne!(cache.lookup(EMBEDDING_LOOKUP_A).unwrap(), "null");
    assert_ne!(cache.lookup(EMBEDDING_LOOKUP_B).unwrap(), "null");
    Python::attach(|py| cache.batch_write(py, WRITE)).unwrap_err();
}
