#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::manager::SharedHealthCheckManager;
use super::status_view::SharedHealthStatusSource;
use super::support::{
    CHECKER, Fault, SHARED_AT, SHARED_SECONDS, World, cached_text, field, instant, keys, number,
};

#[tokio::test]
async fn a_configured_redis_reports_lock_and_cache_state() {
    let world = World::new(instant(SHARED_AT));
    let status = world.manager().get_health_check_status().await.to_json();
    assert_eq!(
        keys(&status),
        vec![
            "pod_id",
            "redis_available",
            "lock_ttl",
            "cache_ttl",
            "lock_owner",
            "lock_in_progress",
            "cache_available",
        ]
    );
    assert_eq!(field(&status, "pod_id"), Json::Text(CHECKER.to_owned()));
    assert_eq!(field(&status, "redis_available"), Json::Bool(true));
    assert_eq!(
        field(&status, "lock_ttl"),
        Json::Number(Number::from(60_i64))
    );
    assert_eq!(
        field(&status, "cache_ttl"),
        Json::Number(Number::from(300_i64))
    );
    assert_eq!(field(&status, "lock_owner"), Json::Null);
    assert_eq!(field(&status, "lock_in_progress"), Json::Bool(false));
    assert_eq!(field(&status, "cache_available"), Json::Bool(false));
}

#[tokio::test]
async fn a_held_lock_names_its_owner() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_7");
    let status = world.manager().get_health_check_status().await.to_json();
    assert_eq!(field(&status, "lock_owner"), Json::Text("pod_7".to_owned()));
    assert_eq!(field(&status, "lock_in_progress"), Json::Bool(true));
    assert_eq!(field(&status, "cache_available"), Json::Bool(false));
}

#[tokio::test]
async fn a_fresh_cache_reports_its_age_and_checker() {
    let world = World::new(instant("2024-01-01T00:01:30Z"));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_9");
    let status = world.manager().get_health_check_status().await.to_json();
    assert_eq!(
        keys(&status),
        vec![
            "pod_id",
            "redis_available",
            "lock_ttl",
            "cache_ttl",
            "lock_owner",
            "lock_in_progress",
            "cache_available",
            "cache_age_seconds",
            "last_checked_by",
        ]
    );
    assert_eq!(field(&status, "lock_owner"), Json::Text("pod_9".to_owned()));
    assert_eq!(field(&status, "lock_in_progress"), Json::Bool(true));
    assert_eq!(field(&status, "cache_available"), Json::Bool(true));
    assert_eq!(field(&status, "cache_age_seconds"), number(90.0));
    assert_eq!(
        field(&status, "last_checked_by"),
        Json::Text("pod_7".to_owned())
    );
}

#[tokio::test]
async fn an_expired_cache_is_reported_unavailable() {
    let world = World::new(instant("2024-01-01T00:05:01Z"));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    let status = world.manager().get_health_check_status().await.to_json();
    assert_eq!(
        keys(&status),
        vec![
            "pod_id",
            "redis_available",
            "lock_ttl",
            "cache_ttl",
            "lock_owner",
            "lock_in_progress",
            "cache_available",
        ]
    );
    assert_eq!(field(&status, "cache_available"), Json::Bool(false));
}

#[tokio::test]
async fn without_redis_only_the_four_shared_keys_are_reported() {
    let world = World::new(instant(SHARED_AT));
    let status = world
        .local_manager()
        .get_health_check_status()
        .await
        .to_json();
    assert_eq!(
        keys(&status),
        vec!["pod_id", "redis_available", "lock_ttl", "cache_ttl"]
    );
    assert_eq!(field(&status, "redis_available"), Json::Bool(false));
    assert_eq!(
        field(&status, "cache_ttl"),
        Json::Number(Number::from(300_i64))
    );
}

#[tokio::test]
async fn a_lock_read_error_is_reported_in_the_status() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Get, SharedHealthCheckManager::LOCK_KEY);
    let status = world.manager().get_health_check_status().await.to_json();
    assert_eq!(
        keys(&status),
        vec![
            "pod_id",
            "redis_available",
            "lock_ttl",
            "cache_ttl",
            "error",
        ]
    );
    assert_eq!(field(&status, "redis_available"), Json::Bool(true));
    let Json::Text(error) = field(&status, "error") else {
        panic!("the status carries the failure");
    };
    assert!(error.contains("async_get_cache"));
}

#[tokio::test]
async fn the_manager_answers_the_status_source() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let source = manager.status().await.expect("the source answers");
    assert_eq!(source, manager.get_health_check_status().await.to_json());
}
