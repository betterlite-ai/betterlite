#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::decode::CachedValue;

use super::manager::SharedHealthCheckManager;
use super::settings::{DEFAULT_SETTINGS, SharedHealthSettings};
use super::support::{Fault, FixedClock, SHARED_AT, SetCall, World, instant};

#[tokio::test]
async fn set_nx_stores_the_pod_id_with_the_lock_ttl() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    assert!(manager.acquire_health_check_lock().await);
    assert_eq!(
        world.cache.sets(),
        vec![SetCall {
            key: SharedHealthCheckManager::LOCK_KEY.to_owned(),
            value: manager.pod_id.clone(),
            ttl: 60,
            nx: true,
        }]
    );
    assert_eq!(
        world.cache.value(SharedHealthCheckManager::LOCK_KEY),
        Some(CachedValue::Text(manager.pod_id.clone()))
    );
    assert!(world.cache.deletes().is_empty());
}

#[tokio::test]
async fn a_lock_held_by_another_pod_cannot_be_taken() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    assert!(manager.acquire_health_check_lock().await);
    let cache = Arc::clone(&world.cache);
    let later = SharedHealthCheckManager::new(
        Some(cache),
        DEFAULT_SETTINGS,
        Arc::new(FixedClock(instant("2024-01-01T00:00:01Z"))),
    );
    assert_ne!(later.pod_id, manager.pod_id);
    assert!(!later.acquire_health_check_lock().await);
    assert_eq!(
        world.cache.value(SharedHealthCheckManager::LOCK_KEY),
        Some(CachedValue::Text(manager.pod_id.clone()))
    );
}

#[tokio::test]
async fn the_configured_lock_ttl_rides_on_the_set() {
    let world = World::new(instant(SHARED_AT));
    let settings = SharedHealthSettings {
        health_check_ttl: 300,
        lock_ttl: 12,
    };
    let manager = world.manager_with(settings);
    assert!(manager.acquire_health_check_lock().await);
    assert_eq!(world.cache.sets()[0].ttl, 12);
}

#[tokio::test]
async fn no_redis_never_acquires_a_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.local_manager();
    assert!(!manager.acquire_health_check_lock().await);
    assert!(world.ops.entries().is_empty());
}

#[tokio::test]
async fn a_cache_error_reads_as_not_acquired() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Set, SharedHealthCheckManager::LOCK_KEY);
    let manager = world.manager();
    assert!(!manager.acquire_health_check_lock().await);
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
}
