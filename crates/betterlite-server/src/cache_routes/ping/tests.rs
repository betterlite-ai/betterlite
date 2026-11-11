#![cfg(test)]

use std::error::Error as _;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use betterlite_cache::types::ping::{HealthCheckCacheParams, TextOrNumber};
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::super::support_tests::{
    ErrorBody, FakeCache, FakeLive, PingBody, admin_router, call_json, fake_cache,
    parsed_cache_params, ping_failure,
};

fn health() -> HealthCheckCacheParams {
    HealthCheckCacheParams {
        host: None,
        port: None,
        redis_kwargs: None,
        namespace: Some("ns".to_owned()),
        redis_version: Some(TextOrNumber::Number(Number::from(7_u32))),
    }
}

#[tokio::test]
async fn uninitialized_cache_answers_503_with_the_cache_ping_error() {
    let (status, body): (StatusCode, ErrorBody) =
        call_json(admin_router(FakeLive::without()), "GET", "/cache/ping", "").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.error.error_type, "cache_ping_error");
    assert_eq!(body.error.param, "cache_ping");
    assert_eq!(body.error.code, "503");
    assert_eq!(
        body.error.message,
        r#"{"message": "Cache not initialized. litellm.cache is None", "litellm_cache_params": "{}", "health_check_cache_params": "{}"}"#
    );
}

#[tokio::test]
async fn a_redis_ping_reports_healthy_with_the_test_write() {
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.params = r#"{"type": "redis"}"#.to_owned();
    cache.health = health();
    let case = Arc::new(cache);
    let (status, body): (StatusCode, PingBody) = call_json(
        admin_router(FakeLive::with(Arc::<FakeCache>::clone(&case))),
        "GET",
        "/cache/ping",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.status, "healthy");
    assert_eq!(body.cache_type, "redis");
    assert_eq!(body.ping_response, Some(true));
    assert_eq!(body.set_cache_response.as_deref(), Some("success"));
    let params = parsed_cache_params(body.litellm_cache_params.as_deref().unwrap());
    assert_eq!(params.get("type"), Some(&Json::Text("redis".to_owned())));
    let returned = body.health_check_cache_params.expect("the health params");
    assert_eq!(returned.namespace.as_deref(), Some("ns"));
    assert_eq!(
        returned.redis_version,
        Some(TextOrNumber::Number(Number::from(7_u32)))
    );
    assert_eq!(case.pings.load(Ordering::SeqCst), 1);
    assert_eq!(case.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_redis_ping_answers_503_and_skips_the_test_write() {
    let empty_health = HealthCheckCacheParams {
        host: None,
        port: None,
        redis_kwargs: None,
        namespace: None,
        redis_version: None,
    };
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.params = r#"{"type": "redis"}"#.to_owned();
    cache.health = empty_health.clone();
    cache.ping = Mutex::new(Some(Err(())));
    let case = Arc::new(cache);
    let (status, body): (StatusCode, ErrorBody) = call_json(
        admin_router(FakeLive::with(Arc::<FakeCache>::clone(&case))),
        "GET",
        "/cache/ping",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body.error.error_type, "cache_ping_error");
    assert_eq!(body.error.param, "cache_ping");
    assert_eq!(body.error.code, "503");
    let health_text = serde_json::to_string(&empty_health).unwrap();
    let expected = format!(
        r#"{{"message": "Service Unhealthy", "litellm_cache_params": {}, "health_check_cache_params": {}}}"#,
        serde_json::to_string(r#"{"type": "redis"}"#).unwrap(),
        serde_json::to_string(&health_text).unwrap()
    );
    assert_eq!(body.error.message, expected);
    assert_eq!(case.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_non_redis_ping_omits_the_ping_and_health_members() {
    let case = Arc::new(fake_cache());
    let (status, body): (StatusCode, PingBody) = call_json(
        admin_router(FakeLive::with(Arc::<FakeCache>::clone(&case))),
        "GET",
        "/cache/ping",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.status, "healthy");
    assert_eq!(body.cache_type, "local");
    assert_eq!(body.ping_response, None);
    assert_eq!(body.set_cache_response, None);
    assert_eq!(body.health_check_cache_params, None);
    let params = parsed_cache_params(body.litellm_cache_params.as_deref().unwrap());
    assert_eq!(params.get("type"), Some(&Json::Text("local".to_owned())));
    assert_eq!(case.pings.load(Ordering::SeqCst), 0);
    assert_eq!(case.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_ping_response_carries_the_backend_boolean() {
    let mut cache = fake_cache();
    cache.kind = "redis".to_owned();
    cache.ping = Mutex::new(Some(Ok(false)));
    let (status, body): (StatusCode, PingBody) = call_json(
        admin_router(FakeLive::with(Arc::new(cache))),
        "GET",
        "/cache/ping",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.ping_response, Some(false));
}

#[test]
fn the_ping_error_carries_the_lifecycle_cause() {
    let error = ping_failure();
    assert!(error.source().is_some());
}
