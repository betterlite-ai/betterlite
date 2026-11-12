#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::scope::{
    active_request_redis_batch, request_redis_batch_scope,
};

use super::tests::support::{FakeRedis, connect_cache, local_memory, settings_for_tests, write};
use crate::backend::memory::local::LocalMemoryOps;
use crate::dual_cache::{DualCache, Ttl};

const DEADLINE: Duration = Duration::from_millis(500);

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

async fn with_redis(server: &FakeRedis) -> DualCache<LocalMemoryOps> {
    let settings = settings_for_tests(Some(30.0_f64));
    DualCache::new(
        local_memory(1000.0),
        Some(connect_cache(server.addr).await),
        settings,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_pre_call_writes_memory_and_queues_the_set() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0_f64)),
    );
    let (batches, deferred) = request_redis_batch_scope(DEADLINE, async {
        let deferred = cache
            .set_cache_pre_call("key", &text("value"), None)
            .expect("the pre-call set");
        assert_eq!(cache.get_cache("key", true), Some(text("value")));
        assert!(server.commands("SET").is_empty());
        let queued = active_request_redis_batch(&redis).expect("the request batch");
        assert_eq!(queued.pending(), 1);
        deferred
    })
    .await;
    let queued = deferred.expect("the request pipeline takes the write");
    assert!(server.commands("SET").is_empty());
    let settled = queued.await;
    assert!(settled.is_ok());
    drop(batches);
    let sets = server.commands("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].args, vec!["SET", "key", "\"value\"", "EX", "30"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_pre_call_without_a_batch_returns_none() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    let deferred = cache
        .set_cache_pre_call("key", &text("value"), None)
        .expect("the fallback decision");
    assert!(deferred.is_none());
    assert_eq!(cache.get_cache("key", true), None);
    assert!(server.commands("SET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_pre_call_keeps_an_explicit_ttl_over_the_default() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    let (batches, deferred) = request_redis_batch_scope(DEADLINE, async {
        cache
            .set_cache_pre_call("key", &text("value"), Some(5.0_f64))
            .expect("the pre-call set")
    })
    .await;
    let queued = deferred.expect("the request pipeline takes the write");
    let settled = queued.await;
    assert!(settled.is_ok());
    drop(batches);
    let sets = server.commands("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].args, vec!["SET", "key", "\"value\"", "EX", "5"]);
    assert_eq!(
        cache.async_get_ttl("key").await.expect("the memory ttl"),
        Some(Ttl::Expiry(1005.0_f64))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_pre_call_deletes_memory_and_queues_the_delete() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(None),
    );
    cache.set_cache("key", &text("value"), write(None, true));
    let (batches, deferred) = request_redis_batch_scope(DEADLINE, async {
        let deferred = cache
            .delete_cache_pre_call("key")
            .expect("the pre-call delete");
        assert_eq!(cache.get_cache("key", true), None);
        assert!(server.commands("DEL").is_empty());
        let queued = active_request_redis_batch(&redis).expect("the request batch");
        assert_eq!(queued.pending(), 1);
        deferred
    })
    .await;
    let queued = deferred.expect("the request pipeline takes the delete");
    let settled = queued.await;
    assert!(settled.is_ok());
    drop(batches);
    let deletes = server.commands("DEL");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].args, vec!["DEL", "key"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_pre_call_without_a_batch_returns_none() {
    let server = FakeRedis::start().await;
    let cache = with_redis(&server).await;
    cache.set_cache("key", &text("value"), write(None, true));
    let deferred = cache
        .delete_cache_pre_call("key")
        .expect("the fallback decision");
    assert!(deferred.is_none());
    assert_eq!(cache.get_cache("key", true), Some(text("value")));
    assert!(server.commands("DEL").is_empty());
}
