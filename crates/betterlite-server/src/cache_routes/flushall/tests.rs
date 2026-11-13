#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::http::StatusCode;

use super::super::admin::RedisAdmin;
use super::super::redis_support::{FakeRedis, PanickingRedis, fake_redis};
use super::super::support_tests::{
    DetailBody, FakeLive, StatusBody, admin_router, call_json, fake_cache,
};

#[tokio::test]
async fn a_redis_cache_is_flushed() {
    let redis = Arc::new(fake_redis());
    let admin: Arc<dyn RedisAdmin> = Arc::<FakeRedis>::clone(&redis);
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.redis = Some(admin);
    let (status, body): (StatusCode, StatusBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "POST",
        "/cache/flushall",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.status, "success");
    assert_eq!(redis.flushes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_uninitialized_cache_answers_503() {
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::without()),
        "POST",
        "/cache/flushall",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.detail,
        "Service Unhealthy (Cache not initialized. litellm.cache is None)"
    );
}

#[tokio::test]
async fn a_non_redis_cache_answers_503() {
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(fake_cache()))),
        "POST",
        "/cache/flushall",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.detail,
        "Service Unhealthy (Cache type local does not support flushing)"
    );
}

#[tokio::test]
async fn a_flush_failure_answers_503() {
    let redis = Arc::new(fake_redis());
    *redis.flush.lock().unwrap() = Err(());
    let admin: Arc<dyn RedisAdmin> = Arc::<FakeRedis>::clone(&redis);
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.redis = Some(admin);
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "POST",
        "/cache/flushall",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.detail, "Service Unhealthy (flushing the cache failed)");
}

#[tokio::test]
async fn a_panicking_flush_answers_503() {
    let admin: Arc<dyn RedisAdmin> = Arc::new(PanickingRedis);
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.redis = Some(admin);
    let (status, body): (StatusCode, DetailBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "POST",
        "/cache/flushall",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body.detail
            .starts_with("Service Unhealthy (a blocking cache call did not finish")
    );
}
