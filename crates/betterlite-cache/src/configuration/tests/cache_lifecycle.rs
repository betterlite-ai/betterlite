#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;

use super::support::{
    FakeRedis, closed_port, dependencies, environment, redis_settings, settings, test_backend,
};
use crate::backend::CacheBackend;
use crate::configuration::cache::{AsyncCacheSupport as _, Cache, CacheLifecycleError};
use crate::configuration::callback_registration::CacheCallbacks;
use crate::types::cache_type::LiteLlmCacheType;

async fn open_local() -> Cache {
    let mut callbacks = CacheCallbacks::default();
    Cache::open(
        settings(LiteLlmCacheType::Local),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a local cache")
}

async fn open_disk() -> (tempfile::TempDir, Cache) {
    let directory = tempfile::tempdir().expect("a temp dir");
    let mut disk = settings(LiteLlmCacheType::Disk);
    disk.disk_cache_dir = Some(directory.path().to_owned());
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(disk, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("a disk cache");
    (directory, cache)
}

async fn open_injected(backend: Arc<dyn CacheBackend>) -> Cache {
    let mut injected = settings(LiteLlmCacheType::Local);
    injected.backend = Some(backend);
    let mut callbacks = CacheCallbacks::default();
    Cache::open(injected, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("an injected cache")
}

async fn open_unreachable_redis() -> Cache {
    let mut unreachable = settings(LiteLlmCacheType::Redis);
    unreachable.redis.host = Some("127.0.0.1".to_owned());
    unreachable.redis.port = Some(IntOrText::Value(i64::from(closed_port().await)));
    let mut callbacks = CacheCallbacks::default();
    Cache::open(unreachable, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("a redis cache")
}

#[tokio::test]
async fn supports_async_is_true_for_every_backend() {
    let local = open_local().await;
    assert!(local.supports_async());
    let (directory, disk) = open_disk().await;
    assert!(disk.supports_async());
    drop(directory);
    let (injected_backend, _disconnects) = test_backend();
    let injected = open_injected(injected_backend).await;
    assert!(injected.supports_async());
    let server = FakeRedis::start().await;
    let mut callbacks = CacheCallbacks::default();
    let redis = Cache::open(
        redis_settings(&server),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a redis cache");
    assert!(redis.supports_async());
}

#[tokio::test]
async fn ping_is_unavailable_for_backends_without_the_operation() {
    let local = open_local().await;
    assert_eq!(local.ping().await.expect("no ping error"), None);
    let (directory, disk) = open_disk().await;
    assert_eq!(disk.ping().await.expect("no ping error"), None);
    drop(directory);
    let (injected_backend, _disconnects) = test_backend();
    let injected = open_injected(injected_backend).await;
    assert_eq!(injected.ping().await.expect("no ping error"), None);
}

#[tokio::test]
async fn ping_delegates_to_the_redis_backend() {
    let server = FakeRedis::start().await;
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        redis_settings(&server),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a redis cache");
    // the first ping waits for the health ping the constructor started, so the count below is settled
    assert_eq!(cache.ping().await.expect("a pong"), Some(true));
    let pings_before = server.commands_named("PING").len();
    assert_eq!(cache.ping().await.expect("a pong"), Some(true));
    assert!(server.commands_named("PING").len() > pings_before);
}

#[tokio::test]
async fn ping_reports_a_redis_connection_failure() {
    let cache = open_unreachable_redis().await;
    let Err(error) = cache.ping().await else {
        panic!("a ping failure");
    };
    assert!(matches!(error, CacheLifecycleError::Ping { .. }));
}

#[tokio::test]
async fn delete_cache_keys_delegates_to_the_redis_backend() {
    let server = FakeRedis::start().await;
    let mut redis = redis_settings(&server);
    redis.namespace = Some("models".to_owned());
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(redis, &mut callbacks, &environment(), &dependencies())
        .await
        .expect("a redis cache");
    cache
        .delete_cache_keys(&["alpha".to_owned(), "beta".to_owned()])
        .await
        .expect("keys deleted");
    let deletes = server.commands_named("DEL");
    assert_eq!(deletes.len(), 1);
    assert_eq!(
        deletes[0].arguments,
        vec![
            "DEL".to_owned(),
            "models:alpha".to_owned(),
            "models:beta".to_owned()
        ]
    );
}

#[tokio::test]
async fn delete_cache_keys_is_a_noop_for_other_backends() {
    let local = open_local().await;
    local
        .delete_cache_keys(&["alpha".to_owned()])
        .await
        .expect("keys deleted");
    let (injected_backend, _disconnects) = test_backend();
    let injected = open_injected(injected_backend).await;
    injected
        .delete_cache_keys(&["alpha".to_owned()])
        .await
        .expect("keys deleted");
}

#[tokio::test]
async fn delete_cache_keys_reports_a_redis_failure() {
    let server = FakeRedis::start().await;
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        redis_settings(&server),
        &mut callbacks,
        &environment(),
        &super::support::rejecting_dependencies(),
    )
    .await
    .expect("a redis cache");
    let Err(error) = cache.delete_cache_keys(&["alpha".to_owned()]).await else {
        panic!("a delete failure");
    };
    assert!(matches!(error, CacheLifecycleError::DeleteKeys { .. }));
}

#[tokio::test]
async fn disconnect_succeeds_for_memory_and_disk() {
    let local = open_local().await;
    local.disconnect().await.expect("disconnected");
    let (directory, disk) = open_disk().await;
    disk.disconnect().await.expect("disconnected");
    drop(directory);
}

#[tokio::test]
async fn disconnect_calls_the_injected_backend() {
    let (backend, disconnects) = test_backend();
    let cache = open_injected(backend).await;
    cache.disconnect().await.expect("disconnected");
    assert_eq!(disconnects.count(), 1);
}

#[tokio::test]
async fn disconnect_releases_the_redis_client_so_the_next_ping_reconnects() {
    let server = FakeRedis::start().await;
    let mut callbacks = CacheCallbacks::default();
    let cache = Cache::open(
        redis_settings(&server),
        &mut callbacks,
        &environment(),
        &dependencies(),
    )
    .await
    .expect("a redis cache");
    cache.ping().await.expect("the first pong");
    let before = server.connections();
    cache.disconnect().await.expect("disconnected");
    cache.ping().await.expect("the second pong");
    assert!(server.connections() > before);
}
