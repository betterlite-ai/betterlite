use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::scope::{
    active_request_redis_batch, request_redis_batch_scope,
};
use serde_json::Number;
use tracing::instrument::WithSubscriber as _;

use super::deferred_log::{capture_warnings, captured};

use super::support::{FakeRedis, Reply, connect_cache, local_memory, settings_for_tests, write};
use crate::dual_cache::{DualCache, DualCacheError, MAX_REDIS_BATCH_CACHE_SIZE, Ttl, lock_read};
use crate::types::pipeline::RedisPipelineIncrement;

const DEADLINE: Duration = Duration::from_millis(500);

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_redis_cache_keeps_the_first_tier() {
    let server = FakeRedis::start().await;
    let first = connect_cache(server.addr).await;
    let second = connect_cache(server.addr).await;
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(None));
    cache.attach_redis_cache(None, Some(5.0_f64));
    assert!(cache.redis().is_none());
    cache.attach_redis_cache(Some(Arc::clone(&first)), Some(5.0_f64));
    assert!(Arc::ptr_eq(&cache.redis().expect("a Redis tier"), &first));
    assert_eq!(*lock_read(&cache.default_redis_ttl), Some(5.0_f64));
    cache.attach_redis_cache(Some(Arc::clone(&second)), Some(9.0_f64));
    assert!(Arc::ptr_eq(&cache.redis().expect("a Redis tier"), &first));
    assert_eq!(*lock_read(&cache.default_redis_ttl), Some(5.0_f64));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_cache_falls_back_to_redis_and_backfills_memory() {
    let server = FakeRedis::start().await;
    server.script("GET", vec![Reply::Bulk("\"from-redis\"".to_owned())]);
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(Some(30.0_f64)),
    );
    assert_eq!(cache.get_cache("key", false), Some(text("from-redis")));
    assert_eq!(
        cache.async_get_ttl("key").await.expect("the backfill ttl"),
        Some(Ttl::Expiry(1030.0_f64))
    );
    assert_eq!(cache.get_cache("key", false), Some(text("from-redis")));
    assert_eq!(server.commands("GET").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_get_cache_falls_back_to_redis_and_backfills_memory() {
    let server = FakeRedis::start().await;
    server.script("GET", vec![Reply::Bulk("\"from-redis\"".to_owned())]);
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(Some(30.0_f64)),
    );
    assert_eq!(
        cache.async_get_cache("key", false).await,
        Some(text("from-redis"))
    );
    assert_eq!(
        cache.async_get_ttl("key").await.expect("the backfill ttl"),
        Some(Ttl::Expiry(1030.0_f64))
    );
    assert_eq!(cache.get_cache("key", true), Some(text("from-redis")));
    assert_eq!(server.commands("GET").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_get_cache_returns_none_when_the_batch_marked_the_key_missing() {
    let server = FakeRedis::start().await;
    server.script("GET", vec![Reply::Bulk("from-redis".to_owned())]);
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(None),
    );
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        let batch = active_request_redis_batch(&redis).expect("a request batch");
        let _misses = batch.mget(&["key".to_owned()]).await;
        assert_eq!(cache.async_get_cache("key", false).await, None);
    })
    .await;
    drop(batches);
    assert!(server.commands("GET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_cache_writes_the_resolved_ttl_to_redis() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(Some(30.0_f64)),
    );
    cache.set_cache("key", &text("value"), write(None, false));
    let sets = server.commands("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].args, vec!["SET", "key", "\"value\"", "EX", "30"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_cache_with_local_only_writes_memory_alone() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    cache.set_cache("key", &text("value"), write(None, true));
    assert!(server.commands("SET").is_empty());
    assert_eq!(cache.get_cache("key", true), Some(text("value")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_set_cache_pipeline_applies_one_ttl_to_every_entry() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(Some(30.0_f64)),
    );
    let entries = vec![
        ("first".to_owned(), text("one")),
        ("second".to_owned(), text("two")),
    ];
    cache
        .async_set_cache_pipeline(&entries, write(None, false))
        .await;
    let sets = server.commands("SET");
    assert_eq!(sets.len(), 2);
    assert!(
        sets.iter()
            .all(|set| set.args.last().map(String::as_str) == Some("30"))
    );
    assert_eq!(
        cache.async_get_ttl("first").await.expect("the first ttl"),
        Some(Ttl::Expiry(1030.0_f64))
    );
    assert_eq!(
        cache.async_get_ttl("second").await.expect("the second ttl"),
        Some(Ttl::Expiry(1030.0_f64))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_set_cache_pipeline_swallows_a_redis_error() {
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
    assert_eq!(cache.get_cache("first", true), Some(text("one")));
    assert_eq!(cache.get_cache("second", true), Some(text("two")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_delete_cache_keys_chunks_the_redis_deletes() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let keys: Vec<String> = (0..=MAX_REDIS_BATCH_CACHE_SIZE)
        .map(|index| format!("key-{index}"))
        .collect();
    let entries: Vec<(String, Json)> = keys
        .iter()
        .map(|key| (key.clone(), text("value")))
        .collect();
    cache
        .async_set_cache_pipeline(&entries, write(None, true))
        .await;
    cache
        .async_delete_cache_keys(&keys)
        .await
        .expect("the deletion");
    let deleted: Vec<usize> = server
        .commands("DEL")
        .iter()
        .map(|command| command.args.len().saturating_sub(1))
        .collect();
    assert_eq!(deleted, vec![MAX_REDIS_BATCH_CACHE_SIZE, 1]);
    assert!(keys.iter().all(|key| cache.get_cache(key, true).is_none()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn increment_cache_propagates_the_redis_error() {
    let server = FakeRedis::start().await;
    server.script("INCRBY", vec![Reply::Error("ERR boom")]);
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let error = cache
        .increment_cache("key", 2.0_f64, write(None, false))
        .expect_err("Redis rejected the increment");
    assert!(matches!(error, DualCacheError::RedisIncrement { .. }));
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::Number(
            Number::from_f64(2.0_f64).expect("a finite number")
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn increment_cache_rejects_a_fractional_increment_for_redis() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let error = cache
        .increment_cache("key", 1.5_f64, write(None, false))
        .expect_err("Redis can only add whole numbers");
    assert!(matches!(error, DualCacheError::FractionalIncrement { .. }));
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::Number(
            Number::from_f64(1.5_f64).expect("a finite number")
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_increment_cache_falls_back_on_the_redis_error() {
    let server = FakeRedis::start().await;
    server.script("INCRBYFLOAT", vec![Reply::Error("ERR boom")]);
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    assert_eq!(
        cache
            .async_increment_cache("key", 2.5_f64, write(None, false))
            .await,
        Some(2.5_f64)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_set_cache_pre_call_returns_the_batch_inside_a_request_scope() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0_f64)),
    );
    let (batches, deferred) = request_redis_batch_scope(DEADLINE, async {
        cache
            .set_cache_pre_call("key", &text("value"), None)
            .expect("the pre-call set")
    })
    .await;
    let queued = deferred.expect("the request pipeline takes the write");
    let _settled = queued.await;
    drop(batches);
    let sets = server.commands("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].args, vec!["SET", "key", "\"value\"", "EX", "30"]);
    assert_eq!(cache.get_cache("key", true), Some(text("value")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_delete_cache_pre_call_defers_the_redis_delete() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(None),
    );
    cache.set_cache("key", &text("value"), write(None, true));
    let (batches, deferred) = request_redis_batch_scope(DEADLINE, async {
        cache
            .delete_cache_pre_call("key")
            .expect("the pre-call delete")
    })
    .await;
    let queued = deferred.expect("the request pipeline takes the delete");
    let _settled = queued.await;
    drop(batches);
    assert_eq!(cache.get_cache("key", true), None);
    let deletes = server.commands("DEL");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].args, vec!["DEL", "key"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_increment_cache_post_call_queues_the_post_call_increment() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(None),
    );
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        cache.increment_cache_post_call("key", 2.0_f64, None).await;
    })
    .await;
    assert!(server.commands("INCRBYFLOAT").is_empty());
    batches.flush_post_call().await;
    let increments = server.commands("INCRBYFLOAT");
    assert_eq!(increments.len(), 1);
    assert_eq!(
        increments[0].args.first().map(String::as_str),
        Some("INCRBYFLOAT")
    );
    assert_eq!(increments[0].args.get(1).map(String::as_str), Some("key"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_set_cache_sadd_writes_memory_and_redis() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    cache
        .async_set_cache_sadd(
            "key",
            &["a".to_owned(), "b".to_owned()],
            write(Some(5.0_f64), false),
        )
        .await
        .expect("the set add");
    let sadds = server.commands("SADD");
    assert_eq!(sadds.len(), 1);
    assert_eq!(sadds[0].args, vec!["SADD", "key", "a", "b"]);
    assert_eq!(
        cache.get_cache("key", true),
        Some(Json::List(vec![text("a"), text("b")]))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flush_and_delete_reach_the_redis_tier() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    cache.set_cache("key", &text("value"), write(None, true));
    cache.delete_cache("key").expect("the delete");
    assert_eq!(server.commands("DEL").len(), 1);
    cache.flush_cache().expect("the flush");
    assert_eq!(server.commands("FLUSHALL").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_delete_cache_reaches_the_redis_tier() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    cache.set_cache("key", &text("value"), write(None, true));
    cache.async_delete_cache("key").await.expect("the delete");
    assert_eq!(server.commands("DEL").len(), 1);
    assert_eq!(cache.get_cache("key", true), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_get_ttl_falls_back_to_redis_on_a_memory_miss() {
    let server = FakeRedis::start().await;
    server.script("TTL", vec![Reply::Integer(42)]);
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    assert_eq!(
        cache.async_get_ttl("key").await.expect("the Redis ttl"),
        Some(Ttl::Seconds(42))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_increment_post_call_queues_every_increment() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(None),
    );
    let increments = [
        RedisPipelineIncrement {
            key: "first".to_owned(),
            increment_value: 1.0_f64,
            ttl: None,
        },
        RedisPipelineIncrement {
            key: "second".to_owned(),
            increment_value: 2.0_f64,
            ttl: None,
        },
    ];
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        cache.pipeline_increment_post_call(&increments).await;
    })
    .await;
    assert!(server.commands("INCRBYFLOAT").is_empty());
    batches.flush_post_call().await;
    assert_eq!(server.commands("INCRBYFLOAT").len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_increment_post_call_counts_memory_before_the_flush() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let increments = [
        RedisPipelineIncrement {
            key: "first".to_owned(),
            increment_value: 1.0_f64,
            ttl: None,
        },
        RedisPipelineIncrement {
            key: "second".to_owned(),
            increment_value: 2.0_f64,
            ttl: None,
        },
    ];
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        cache.pipeline_increment_post_call(&increments).await;
        let first = Number::from_f64(1.0_f64).expect("a finite number");
        let second = Number::from_f64(2.0_f64).expect("a finite number");
        assert_eq!(cache.get_cache("first", true), Some(Json::Number(first)));
        assert_eq!(cache.get_cache("second", true), Some(Json::Number(second)));
    })
    .await;
    assert!(server.commands("INCRBYFLOAT").is_empty());
    drop(batches);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_post_call_redis_increment_is_logged_after_the_flush() {
    let server = FakeRedis::start().await;
    server.script("INCRBYFLOAT", vec![Reply::Error("ERR boom")]);
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        cache.increment_cache_post_call("key", 2.0_f64, None).await;
    })
    .await;
    let (subscriber, bytes) = capture_warnings();
    batches.flush_post_call().with_subscriber(subscriber).await;
    let log = captured(&bytes);
    assert!(log.contains("post-call Redis increment failed"), "{log}");
}
