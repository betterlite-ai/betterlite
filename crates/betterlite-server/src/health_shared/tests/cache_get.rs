#![cfg(test)]

use super::manager::SharedHealthCheckManager;
use super::settings::SharedHealthSettings;
use super::support::{
    Fault, SHARED_AT, SHARED_SECONDS, World, cached_text, healthy_row, instant, number,
    unhealthy_row,
};

#[tokio::test]
async fn a_fresh_entry_comes_back_with_its_rows_and_checker() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    let cached = world
        .manager()
        .cached_health_check_results()
        .await
        .expect("the fresh entry is returned");
    assert_eq!(cached.healthy_endpoints, vec![healthy_row("cached-ok")]);
    assert_eq!(
        cached.unhealthy_endpoints,
        vec![unhealthy_row("cached-down", 500)]
    );
    assert_eq!(cached.checked_by.as_deref(), Some("pod_7"));
    assert_eq!(number(cached.timestamp), number(SHARED_SECONDS));
}

#[tokio::test]
async fn a_json_string_decodes_like_an_object() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_plain(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    let from_text = world
        .manager()
        .cached_health_check_results()
        .await
        .expect("the string form decodes");
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_8"),
    );
    let from_object = world
        .manager()
        .cached_health_check_results()
        .await
        .expect("the object form decodes");
    assert_eq!(from_text.healthy_endpoints, from_object.healthy_endpoints);
    assert_eq!(from_text.checked_by.as_deref(), Some("pod_7"));
    assert_eq!(from_object.checked_by.as_deref(), Some("pod_8"));
}

#[tokio::test]
async fn an_entry_exactly_at_the_ttl_is_still_fresh() {
    let world = World::new(instant("2024-01-01T00:05:00Z"));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_some()
    );
}

#[tokio::test]
async fn an_entry_one_second_past_the_ttl_is_expired() {
    let world = World::new(instant("2024-01-01T00:05:01Z"));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn the_configured_ttl_moves_the_boundary() {
    let world = World::new(instant("2024-01-01T00:00:10Z"));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    let at_ten = SharedHealthSettings {
        health_check_ttl: 10,
        lock_ttl: 60,
    };
    assert!(
        world
            .manager_with(at_ten)
            .cached_health_check_results()
            .await
            .is_some()
    );
    let below_ten = SharedHealthSettings {
        health_check_ttl: 9,
        lock_ttl: 60,
    };
    assert!(
        world
            .manager_with(below_ten)
            .cached_health_check_results()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_future_timestamp_is_not_expired() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(1_704_068_200.0, "pod_7"),
    );
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_some()
    );
}

#[tokio::test]
async fn a_missing_entry_is_none() {
    let world = World::new(instant(SHARED_AT));
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_read_error_is_none() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Get, SharedHealthCheckManager::CACHE_KEY);
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn an_unreadable_string_is_none() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::CACHE_KEY, "{not json");
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn an_object_that_is_not_a_result_is_none() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        "{\"healthy_endpoints\":5}",
    );
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn a_payload_without_a_timestamp_has_expired() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        "{\"healthy_endpoints\":[],\"unhealthy_endpoints\":[]}",
    );
    assert!(
        world
            .manager()
            .cached_health_check_results()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn no_redis_reads_nothing() {
    let world = World::new(instant(SHARED_AT));
    assert!(
        world
            .local_manager()
            .cached_health_check_results()
            .await
            .is_none()
    );
    assert!(world.ops.entries().is_empty());
}
