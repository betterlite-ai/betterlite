use serde_json::Number;

use crate::backend::memory::local::LocalMemoryOps;

use betterlite_models::decisions::Json;

use super::support::{FakeRedis, Reply, connect_cache, local_memory, settings_for_tests, write};
use crate::dual_cache::DualCache;
use crate::types::pipeline::RedisPipelineIncrement;

async fn with_redis(server: &FakeRedis) -> DualCache<LocalMemoryOps> {
    DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(Some(30.0_f64)),
    )
}

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

#[tokio::test]
async fn increment_cache_returns_the_redis_count() {
    let server = FakeRedis::start().await;
    server.script("INCRBY", vec![Reply::Integer(7)]);
    let cache = with_redis(&server).await;
    let counted = cache
        .increment_cache("key", 2.0_f64, write(None, false))
        .expect("the Redis increment");
    assert_eq!(counted.to_bits(), 7.0_f64.to_bits());
    assert_eq!(server.commands("INCRBY").len(), 1);
}

#[tokio::test]
async fn increment_cache_with_local_only_skips_redis() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    let counted = cache
        .increment_cache("key", 2.0_f64, write(None, true))
        .expect("the memory increment");
    assert_eq!(counted.to_bits(), 2.0_f64.to_bits());
    assert!(server.commands("INCRBY").is_empty());
}

#[tokio::test]
async fn async_increment_cache_returns_the_redis_count() {
    let server = FakeRedis::start().await;
    server.script("INCRBYFLOAT", vec![Reply::Bulk("9.5".to_owned())]);
    let cache = with_redis(&server).await;
    let counted = cache
        .async_increment_cache("key", 2.0_f64, write(None, false))
        .await;
    assert_eq!(counted, Some(9.5_f64));
}

#[tokio::test]
async fn async_increment_cache_with_local_only_skips_redis() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    let counted = cache
        .async_increment_cache("key", 2.0_f64, write(None, true))
        .await;
    assert_eq!(counted, Some(2.0_f64));
    assert!(server.commands("INCRBYFLOAT").is_empty());
}

#[tokio::test]
async fn async_increment_cache_pipeline_returns_the_redis_counts() {
    let server = FakeRedis::start().await;
    server.script(
        "INCRBYFLOAT",
        vec![Reply::Bulk("7.5".to_owned()), Reply::Bulk("8.5".to_owned())],
    );
    let cache = with_redis(&server).await;
    let counted = cache
        .async_increment_cache_pipeline(&[pair("first", 1.0_f64), pair("second", 2.5_f64)], false)
        .await;
    assert_eq!(counted, Some(vec![7.5_f64, 8.5_f64]));
}

#[tokio::test]
async fn async_increment_cache_pipeline_falls_back_on_a_redis_error() {
    let server = FakeRedis::start().await;
    server.script(
        "INCRBYFLOAT",
        vec![Reply::Error("ERR boom"), Reply::Error("ERR boom")],
    );
    let cache = with_redis(&server).await;
    let counted = cache
        .async_increment_cache_pipeline(&[pair("first", 1.0_f64), pair("second", 2.5_f64)], false)
        .await;
    assert_eq!(counted, Some(vec![1.0_f64, 2.5_f64]));
}

#[tokio::test]
async fn async_increment_cache_pipeline_with_local_only_skips_redis() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    let counted = cache
        .async_increment_cache_pipeline(&[pair("first", 1.0_f64)], true)
        .await;
    assert_eq!(counted, Some(vec![1.0_f64]));
    assert!(server.commands("INCRBYFLOAT").is_empty());
}

#[tokio::test]
async fn async_set_cache_writes_the_resolved_ttl_to_redis() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    cache
        .async_set_cache("key", &text("value"), write(None, false))
        .await;
    let sets = server.commands("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].args, vec!["SET", "key", "\"value\"", "EX", "30"]);
    assert_eq!(cache.get_cache("key", true), Some(text("value")));
}

#[tokio::test]
async fn async_set_cache_with_local_only_writes_memory_alone() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    cache
        .async_set_cache("key", &text("value"), write(None, true))
        .await;
    assert!(server.commands("SET").is_empty());
    assert_eq!(cache.get_cache("key", true), Some(text("value")));
}

#[tokio::test]
async fn async_set_cache_pipeline_with_local_only_writes_memory_alone() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    let entries = vec![("key".to_owned(), text("value"))];
    cache
        .async_set_cache_pipeline(&entries, write(None, true))
        .await;
    assert!(server.commands("SET").is_empty());
    assert_eq!(cache.get_cache("key", true), Some(text("value")));
}

#[tokio::test]
async fn async_set_cache_sadd_with_local_only_skips_redis() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    cache
        .async_set_cache_sadd("key", &["a".to_owned()], write(None, true))
        .await
        .expect("the memory set add");
    assert!(server.commands("SADD").is_empty());
}

#[tokio::test]
async fn a_whole_number_round_trips_through_the_memory_tier() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    let whole = Json::Number(Number::from(5_i64));
    cache.set_cache("key", &whole, write(None, true));
    assert_eq!(cache.get_cache("key", true), Some(whole));
}
