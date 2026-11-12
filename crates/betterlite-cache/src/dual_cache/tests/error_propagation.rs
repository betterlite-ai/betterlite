#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::tests::support::{
    FakeRedis, RejectingGate, Reply, connect_cache, connect_cache_with_gate, local_memory,
    settings_for_tests, write,
};
use crate::backend::memory::local::LocalMemoryOps;
use crate::dual_cache::{DualCache, DualCacheError};
use crate::types::pipeline::RedisPipelineIncrement;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn pair(key: &str, amount: f64) -> RedisPipelineIncrement {
    RedisPipelineIncrement {
        key: key.to_owned(),
        increment_value: amount,
        ttl: None,
    }
}

async fn rejecting_cache(server: &FakeRedis) -> DualCache<LocalMemoryOps> {
    DualCache::new(
        local_memory(1000.0),
        Some(connect_cache_with_gate(server.addr, Arc::new(RejectingGate)).await),
        settings_for_tests(None),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_cache_pipeline_swallows_a_redis_failure() {
    let server = FakeRedis::start().await;
    server.script(
        "SET",
        vec![Reply::Error("ERR boom"), Reply::Error("ERR boom")],
    );
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let entries = vec![
        ("first".to_owned(), text("one")),
        ("second".to_owned(), text("two")),
    ];
    cache
        .async_set_cache_pipeline(&entries, write(None, false))
        .await;
    assert_eq!(server.commands("SET").len(), 2);
    assert_eq!(cache.get_cache("first", true), Some(text("one")));
    assert_eq!(cache.get_cache("second", true), Some(text("two")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn increment_cache_pipeline_swallows_a_redis_failure() {
    let server = FakeRedis::start().await;
    server.script(
        "INCRBYFLOAT",
        vec![Reply::Error("ERR boom"), Reply::Error("ERR boom")],
    );
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let increments = [pair("first", 1.0_f64), pair("second", 2.5_f64)];
    let results = cache
        .async_increment_cache_pipeline(&increments, false)
        .await;
    assert_eq!(results, Some(vec![1.0_f64, 2.5_f64]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn increment_cache_pipeline_returns_none_when_memory_fails() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    cache.set_cache("first", &text("not a number"), write(None, true));
    let results = cache
        .async_increment_cache_pipeline(&[pair("first", 1.0_f64)], false)
        .await;
    assert_eq!(results, None);
    assert!(server.commands("INCRBYFLOAT").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_cache_pipeline_swallows_an_open_circuit_breaker() {
    let server = FakeRedis::start().await;
    let cache = rejecting_cache(&server).await;
    let entries = vec![("key".to_owned(), text("value"))];
    cache
        .async_set_cache_pipeline(&entries, write(None, false))
        .await;
    assert_eq!(cache.get_cache("key", true), Some(text("value")));
    assert!(server.commands("SET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn increment_cache_pipeline_falls_back_on_an_open_circuit_breaker() {
    let server = FakeRedis::start().await;
    let cache = rejecting_cache(&server).await;
    let results = cache
        .async_increment_cache_pipeline(&[pair("first", 1.0_f64)], false)
        .await;
    assert_eq!(results, Some(vec![1.0_f64]));
    assert!(server.commands("INCRBYFLOAT").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_cache_sadd_propagates_an_open_circuit_breaker() {
    let server = FakeRedis::start().await;
    let cache = rejecting_cache(&server).await;
    let error = cache
        .async_set_cache_sadd("key", &["a".to_owned()], write(None, false))
        .await
        .expect_err("the open breaker reaches the caller");
    assert!(matches!(error, DualCacheError::RedisSetAdd { .. }));
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::List(vec![text("a")]))
    );
}
