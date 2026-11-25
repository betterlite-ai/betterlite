#![cfg(test)]

use super::manager::SharedHealthCheckManager;
use super::status_view::{SharedHealthStatus, SharedHealthStatusSource as _};
use super::support::{Fault, SHARED_AT, SHARED_SECONDS, World, cached_text, instant};

fn status_text(status: &SharedHealthStatus) -> String {
    serde_json::to_string(status).expect("the status serializes")
}

#[tokio::test]
async fn a_configured_redis_reports_lock_and_cache_state() {
    let world = World::new(instant(SHARED_AT));
    let status = world.manager().get_health_check_status().await;
    assert_eq!(
        status_text(&status),
        r#"{"pod_id":"pod_1704067200000","redis_available":true,"lock_ttl":60,"cache_ttl":300,"lock_owner":null,"lock_in_progress":false,"cache_available":false}"#
    );
}

#[tokio::test]
async fn a_held_lock_names_its_owner() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_7");
    let status = world.manager().get_health_check_status().await;
    assert_eq!(
        status_text(&status),
        r#"{"pod_id":"pod_1704067200000","redis_available":true,"lock_ttl":60,"cache_ttl":300,"lock_owner":"pod_7","lock_in_progress":true,"cache_available":false}"#
    );
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
    let status = world.manager().get_health_check_status().await;
    assert_eq!(
        status_text(&status),
        r#"{"pod_id":"pod_1704067200000","redis_available":true,"lock_ttl":60,"cache_ttl":300,"lock_owner":"pod_9","lock_in_progress":true,"cache_available":true,"cache_age_seconds":90.0,"last_checked_by":"pod_7"}"#
    );
}

#[tokio::test]
async fn an_expired_cache_is_reported_unavailable() {
    let world = World::new(instant("2024-01-01T00:05:01Z"));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    let status = world.manager().get_health_check_status().await;
    assert_eq!(
        status_text(&status),
        r#"{"pod_id":"pod_1704067200000","redis_available":true,"lock_ttl":60,"cache_ttl":300,"lock_owner":null,"lock_in_progress":false,"cache_available":false}"#
    );
}

#[tokio::test]
async fn without_redis_only_the_four_shared_keys_are_reported() {
    let world = World::new(instant(SHARED_AT));
    let status = world.local_manager().get_health_check_status().await;
    assert_eq!(
        status_text(&status),
        r#"{"pod_id":"pod_1704067200000","redis_available":false,"lock_ttl":60,"cache_ttl":300}"#
    );
}

#[tokio::test]
async fn a_lock_read_error_is_reported_in_the_status() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Get, SharedHealthCheckManager::LOCK_KEY);
    let status = world.manager().get_health_check_status().await;
    assert_eq!(
        status_text(&status),
        r#"{"pod_id":"pod_1704067200000","redis_available":true,"lock_ttl":60,"cache_ttl":300,"error":"the health check cache async_get_cache failed"}"#
    );
}

#[tokio::test]
async fn the_manager_answers_the_status_source() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let source = manager.status().await.expect("the source answers");
    assert_eq!(source, manager.get_health_check_status().await);
}
