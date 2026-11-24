#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use betterlite_cache::backend::{CacheBackend, CacheResult};
use betterlite_cache::configuration::cache::Backend;
use betterlite_cache::types::embedding::CachedEmbedding;
use pyo3::Python;
use pyo3_async_runtimes::tokio::get_runtime;
use serde::Deserialize;

use super::CacheCore;
use crate::cache::convert::{self, CacheCall};

const LOOKUP: &str =
    r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}}"#;

const WRITE: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}, "result": {"content": "hello"}}"#;

const FRESH_LOOKUP: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "cache": {"s-maxage": 100}}}"#;

const STALE_LOOKUP: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "cache": {"s-maxage": -1}}}"#;

const PRESET_LOOKUP: &str =
    r#"{"kwargs": {"model": "gpt-4o", "litellm_params": {"preset_cache_key": "given"}}}"#;

const PRESET_WRITE: &str = r#"{"kwargs": {"model": "gpt-4o", "litellm_params": {"preset_cache_key": "given"}}, "result": {"content": "hello"}}"#;

const CALLER_KEY_LOOKUP: &str = r#"{"kwargs": {"model": "gpt-4o", "cache_key": "caller"}}"#;

const REQUEST_NAMESPACE_LOOKUP: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "cache": {"namespace": "other"}}}"#;

const OPTED_IN_LOOKUP: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "cache": {"use-cache": true}}}"#;

const OPTED_IN_WRITE: &str = r#"{"kwargs": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "cache": {"use-cache": true}}, "result": {"content": "hello"}}"#;

const EMBEDDING_WRITE: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": ["a", "b"]}, "result": {"model": "text-embedding-3-small", "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}, {"object": "embedding", "embedding": [0.3, 0.4], "index": 1}], "usage": {"prompt_tokens": 7}}, "input": "b", "index": 1}"#;

const EMBEDDING_MISMATCH: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": ["a", "b"]}, "result": {"model": "text-embedding-3-small", "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}], "usage": {"prompt_tokens": 3}}}"#;

const EMBEDDING_LOOKUP_A: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": "a"}}"#;

const EMBEDDING_LOOKUP_B: &str = r#"{"kwargs": {"model": "text-embedding-3-small", "input": "b"}}"#;

#[derive(Debug, Deserialize)]
struct EntryAnswer {
    cache_key: String,
    cached_data: EntryEnvelope,
}

#[derive(Debug, Deserialize)]
struct EntryEnvelope {
    response: CachedEmbedding,
}

fn open(settings: &str) -> CacheCore {
    Python::initialize();
    let wire = convert::parse_settings(settings).unwrap();
    get_runtime().block_on(CacheCore::open(&wire)).unwrap()
}

fn call(text: &str) -> CacheCall {
    convert::parse_call(text).unwrap()
}

// An injected backend whose disconnect leaves a mark, so close has something to show.
#[derive(Default)]
struct ClosingBackend {
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl CacheBackend for ClosingBackend {
    fn default_ttl(&self) -> i64 {
        60_i64
    }

    async fn disconnect(&self) -> CacheResult<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn cache_core_rejects_a_type_it_cannot_build() {
    Python::initialize();
    let wire = convert::parse_settings(r#"{"type": "redis"}"#).unwrap();
    let Err(refused) = get_runtime().block_on(CacheCore::open(&wire)) else {
        panic!("a redis cache has no native store");
    };
    assert!(refused.to_string().contains("Redis"));
}

#[test]
fn cache_core_round_trips_a_written_value() {
    let core = open("{}");
    assert_eq!(core.lookup(&call(LOOKUP)).unwrap(), "null");
    core.write(&call(WRITE)).unwrap();
    assert_eq!(
        core.lookup(&call(LOOKUP)).unwrap(),
        r#"{"content":"hello"}"#
    );
}

#[test]
fn cache_core_async_twins_share_the_store() {
    let core = open("{}");
    get_runtime()
        .block_on(core.async_write(&call(WRITE)))
        .unwrap();
    assert_eq!(
        core.lookup(&call(LOOKUP)).unwrap(),
        r#"{"content":"hello"}"#
    );
    let hit = get_runtime()
        .block_on(core.async_lookup(&call(LOOKUP)))
        .unwrap();
    assert_eq!(hit, r#"{"content":"hello"}"#);
    assert_eq!(
        get_runtime()
            .block_on(core.async_lookup(&call(CALLER_KEY_LOOKUP)))
            .unwrap(),
        "null"
    );
}

#[test]
fn cache_core_expires_a_hit_past_s_maxage() {
    let core = open("{}");
    core.write(&call(WRITE)).unwrap();
    assert_eq!(
        core.lookup(&call(FRESH_LOOKUP)).unwrap(),
        r#"{"content":"hello"}"#
    );
    assert_eq!(core.lookup(&call(STALE_LOOKUP)).unwrap(), "null");
}

#[test]
fn cache_core_default_off_requires_the_use_cache_flag() {
    let core = open(r#"{"mode": "default_off"}"#);
    core.write(&call(WRITE)).unwrap();
    assert_eq!(core.lookup(&call(LOOKUP)).unwrap(), "null");
    assert_eq!(core.lookup(&call(OPTED_IN_LOOKUP)).unwrap(), "null");
    core.write(&call(OPTED_IN_WRITE)).unwrap();
    assert_eq!(
        core.lookup(&call(OPTED_IN_LOOKUP)).unwrap(),
        r#"{"content":"hello"}"#
    );
    assert_eq!(core.lookup(&call(LOOKUP)).unwrap(), "null");
}

#[test]
fn cache_core_prefixes_the_namespace_and_honours_a_preset_key() {
    let core = open(r#"{"namespace": "ns"}"#);
    assert!(core.cache_key(&call(LOOKUP)).unwrap().starts_with("ns:"));
    let request_namespace = core.cache_key(&call(REQUEST_NAMESPACE_LOOKUP)).unwrap();
    assert!(request_namespace.starts_with("other:"));
    assert_eq!(core.cache_key(&call(PRESET_LOOKUP)).unwrap(), "given");
    assert_eq!(core.cache_key(&call(CALLER_KEY_LOOKUP)).unwrap(), "caller");
    core.write(&call(PRESET_WRITE)).unwrap();
    assert_eq!(
        core.lookup(&call(PRESET_LOOKUP)).unwrap(),
        r#"{"content":"hello"}"#
    );
}

#[test]
fn cache_core_writes_an_embedding_entry_without_storing_it() {
    let core = open("{}");
    let text = core.write_embedding(&call(EMBEDDING_WRITE)).unwrap();
    let entry: EntryAnswer = serde_json::from_str(&text).unwrap();
    assert_eq!(
        entry.cache_key,
        core.cache_key(&call(EMBEDDING_LOOKUP_B)).unwrap()
    );
    assert_eq!(entry.cached_data.response.format_version, 2);
    assert_eq!(entry.cached_data.response.index, Some(1));
    assert_eq!(entry.cached_data.response.prompt_tokens, Some(3));
    assert_eq!(core.lookup(&call(EMBEDDING_LOOKUP_B)).unwrap(), "null");
}

#[test]
fn cache_core_pipeline_writes_every_input_and_skips_a_count_mismatch() {
    let core = open("{}");
    get_runtime()
        .block_on(core.async_write_pipeline(&call(EMBEDDING_WRITE)))
        .unwrap();
    let first: CachedEmbedding =
        serde_json::from_str(&core.lookup(&call(EMBEDDING_LOOKUP_A)).unwrap()).unwrap();
    let second: CachedEmbedding =
        serde_json::from_str(&core.lookup(&call(EMBEDDING_LOOKUP_B)).unwrap()).unwrap();
    assert_eq!(first.index, Some(0));
    assert_eq!(second.index, Some(1));
    assert_eq!(first.prompt_tokens, Some(4));
    assert_eq!(second.prompt_tokens, Some(3));
}

#[test]
fn cache_core_pipeline_skips_when_the_counts_differ() {
    let core = open("{}");
    get_runtime()
        .block_on(core.async_write_pipeline(&call(EMBEDDING_MISMATCH)))
        .unwrap();
    assert_eq!(core.lookup(&call(EMBEDDING_LOOKUP_A)).unwrap(), "null");
    assert_eq!(core.lookup(&call(EMBEDDING_LOOKUP_B)).unwrap(), "null");
}

#[test]
fn cache_core_batch_write_answers_not_implemented() {
    let core = open("{}");
    let refused = get_runtime()
        .block_on(core.async_batch_write(&call(WRITE)))
        .unwrap_err();
    assert!(refused.is_not_implemented());
}

#[test]
fn cache_core_semantic_lookup_answers_the_response_and_no_similarity() {
    let core = open("{}");
    core.write(&call(WRITE)).unwrap();
    assert_eq!(
        core.lookup_semantic(&call(LOOKUP)).unwrap(),
        r#"[{"content":"hello"},null]"#
    );
    assert_eq!(
        core.lookup_semantic(&call(CALLER_KEY_LOOKUP)).unwrap(),
        "[null,null]"
    );
}

#[test]
fn cache_core_closes_the_backend() {
    let core = open("{}");
    core.write(&call(WRITE)).unwrap();
    get_runtime().block_on(core.close()).unwrap();
}

#[test]
fn cache_core_close_disconnects_the_injected_backend() {
    let mut core = open("{}");
    let backend = Arc::new(ClosingBackend::default());
    let closed = Arc::clone(&backend.closed);
    core.cache.backend = Backend::Injected(backend);
    get_runtime().block_on(core.close()).unwrap();
    assert!(closed.load(Ordering::SeqCst));
}

#[test]
fn cache_core_async_semantic_lookup_reads_the_same_entry() {
    let core = open("{}");
    core.write(&call(WRITE)).unwrap();
    assert_eq!(
        get_runtime()
            .block_on(core.async_lookup_semantic(&call(LOOKUP)))
            .unwrap(),
        r#"[{"content":"hello"},null]"#
    );
}
