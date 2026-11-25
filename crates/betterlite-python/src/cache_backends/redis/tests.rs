#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_cache::configuration::settings::{
    DEFAULT_REDIS_MAJOR_VERSION, DEFAULT_REDIS_TTL_SECONDS,
};
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::RedisCacheOptions;
use betterlite_redis::redis_cache::connect::connect;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use pyo3_async_runtimes::tokio::get_runtime;

use super::*;
use crate::cache_backends::settings::RedisSettings;

// The numeric overrides are text because arbitrary_precision breaks the untagged number fields. Nothing listens on port 1, so a call reaches the client and every command fails on the way out.
const DEAD_REDIS: &str = r#"{"host": "127.0.0.1", "port": "1", "socket_timeout": "0.1", "socket_connect_timeout": "0.1"}"#;

fn dead_redis() -> NativeRedis {
    Python::initialize();
    let overrides: RedisOverrides = serde_json::from_str(DEAD_REDIS).unwrap();
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: DEFAULT_REDIS_TTL_SECONDS,
        redis_flush_size: None,
        default_major_version: DEFAULT_REDIS_MAJOR_VERSION,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(QuietHooks),
        gate: Arc::new(QuietGate),
    };
    let environment = BTreeMap::<String, String>::new();
    let cache: RedisCache = get_runtime()
        .block_on(connect(options, &environment))
        .unwrap();
    NativeRedis {
        cache: Arc::new(cache),
        settings: RedisSettings::default(),
    }
}

#[test]
fn backends_redis_errors_surface() {
    let native = dead_redis();
    Python::attach(|py| {
        native.set(py, "key", "1", None).unwrap();
        assert_eq!(native.get(py, "key").unwrap(), None);
        native.increment(py, "key", 1_i64, None).unwrap_err();
        native.delete(py, vec!["key".to_owned()]).unwrap_err();
        native.flush(py).unwrap_err();
        native.ping(py).unwrap_err();
        assert_eq!(native.ttl(py, "key").unwrap(), None);
    });
}

#[test]
fn backends_redis_close_is_ok() {
    let native = dead_redis();
    Python::attach(|py| native.close(py));
}

// An equal namespace keeps the open client; a rebuilt one would swap the Arc the cache lives in.
#[test]
fn backends_redis_namespace_keeps_the_cache() {
    let mut native = dead_redis();
    let before = Arc::clone(&native.cache);
    Python::attach(|py| native.set_namespace(py, None).unwrap());
    assert!(Arc::ptr_eq(&before, &native.cache));
}

// The constructor helper builds the same lazy client the other tests assemble by hand.
#[test]
fn backends_redis_open_connects() {
    let overrides: RedisOverrides = serde_json::from_str(DEAD_REDIS).unwrap();
    let settings = RedisSettings {
        overrides,
        namespace: None,
        default_ttl: None,
        redis_flush_size: None,
    };
    Python::attach(|py| open(py, settings).unwrap());
}

#[test]
fn backends_redis_answers_the_value_text() {
    assert_eq!(
        answer_text(Some(CachedValue::Null)).unwrap(),
        Some("null".to_owned())
    );
    assert_eq!(answer_text(None).unwrap(), None);
}

#[test]
fn backends_redis_gate_admits() {
    let gate = QuietGate;
    gate.enter("probe").unwrap();
}

#[test]
fn backends_redis_store_swallows_a_failure() {
    let native = dead_redis();
    get_runtime()
        .block_on(store(
            Arc::clone(&native.cache),
            "key".to_owned(),
            Json::Bool(true),
            None,
            false,
        ))
        .unwrap();
}

#[test]
fn backends_redis_count_reports_failure() {
    let native = dead_redis();
    get_runtime()
        .block_on(count(
            Arc::clone(&native.cache),
            "count".to_owned(),
            2.0_f64,
            None,
        ))
        .unwrap_err();
}
