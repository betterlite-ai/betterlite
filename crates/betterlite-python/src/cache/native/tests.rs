#![cfg(test)]

use pyo3::prelude::*;

use super::NativeCache;
use crate::cache::register;

const LOOKUP: &str =
    r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}}"#;

const WRITE: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}, "result": {"content": "hello"}}"#;

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
