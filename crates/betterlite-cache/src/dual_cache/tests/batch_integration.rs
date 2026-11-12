#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::scope::{
    active_post_call_redis_batch, request_redis_batch_scope,
};
use serde_json::Number;
use tracing::instrument::WithSubscriber as _;

use super::tests::deferred_log::{capture_warnings, captured};
use super::tests::support::{
    FakeRedis, Reply, connect_cache, local_memory, settings_for_tests, write,
};
use crate::dual_cache::DualCache;
use crate::types::pipeline::RedisPipelineIncrement;

const DEADLINE: Duration = Duration::from_millis(500);

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite number"))
}

fn pair(key: &str, amount: f64) -> RedisPipelineIncrement {
    RedisPipelineIncrement {
        key: key.to_owned(),
        increment_value: amount,
        ttl: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_call_increment_queues_a_single_increment() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(None),
    );
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        cache
            .increment_cache_post_call("key", 2.0_f64, Some(60))
            .await;
        assert_eq!(cache.get_cache("key", true), Some(number(2.0_f64)));
        assert!(server.commands("INCRBYFLOAT").is_empty());
        let batch = active_post_call_redis_batch(&redis).expect("the post-call batch");
        assert_eq!(batch.pending(), 1);
    })
    .await;
    batches.flush_post_call().await;
    let increments = server.commands("INCRBYFLOAT");
    assert_eq!(increments.len(), 1);
    assert_eq!(increments[0].args, vec!["INCRBYFLOAT", "key", "2.0"]);
    let expiries = server.commands("EXPIRE");
    assert_eq!(expiries.len(), 1);
    assert_eq!(expiries[0].args, vec!["EXPIRE", "key", "60"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_call_increment_without_a_batch_takes_the_direct_path() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    cache.increment_cache_post_call("key", 3.0_f64, None).await;
    let increments = server.commands("INCRBYFLOAT");
    assert_eq!(increments.len(), 1);
    assert_eq!(increments[0].args, vec!["INCRBYFLOAT", "key", "3.0"]);
    assert_eq!(cache.get_cache("key", true), Some(number(3.0_f64)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_post_call_increment_is_logged_after_the_flush() {
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
    assert!(log.contains("WARN"), "{log}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_increment_post_call_logs_each_failed_increment() {
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
    let increments = [pair("first", 1.0_f64), pair("second", 2.0_f64)];
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        cache.pipeline_increment_post_call(&increments).await;
        assert_eq!(cache.get_cache("first", true), Some(number(1.0_f64)));
        assert_eq!(cache.get_cache("second", true), Some(number(2.0_f64)));
    })
    .await;
    let (subscriber, bytes) = capture_warnings();
    batches.flush_post_call().with_subscriber(subscriber).await;
    let log = captured(&bytes);
    assert_eq!(log.matches("post-call Redis increment failed").count(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_increment_post_call_queues_the_redis_increment_when_memory_fails() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    cache.set_cache("first", &text("not a number"), write(None, true));
    let increments = [pair("first", 1.0_f64), pair("second", 2.0_f64)];
    let (subscriber, bytes) = capture_warnings();
    let (batches, ()) = request_redis_batch_scope(DEADLINE, async {
        cache.pipeline_increment_post_call(&increments).await;
    })
    .with_subscriber(subscriber)
    .await;
    let log = captured(&bytes);
    assert!(log.contains("in-memory increment failed"), "{log}");
    assert_eq!(cache.get_cache("first", true), Some(text("not a number")));
    batches.flush_post_call().await;
    assert_eq!(server.commands("INCRBYFLOAT").len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_increment_post_call_without_a_batch_runs_the_direct_pipeline() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let increments = [pair("first", 1.0_f64), pair("second", 2.5_f64)];
    cache.pipeline_increment_post_call(&increments).await;
    assert_eq!(server.commands("INCRBYFLOAT").len(), 2);
    assert_eq!(cache.get_cache("first", true), Some(number(1.0_f64)));
    assert_eq!(cache.get_cache("second", true), Some(number(2.5_f64)));
}
