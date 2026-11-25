#![cfg(test)]

use super::manager::SharedHealthCheckManager;
use super::support::{Fault, SHARED_AT, World, instant};

#[tokio::test]
async fn the_owner_deletes_its_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    assert!(manager.acquire_health_check_lock().await);
    manager.release_health_check_lock().await;
    assert_eq!(
        world.cache.deletes(),
        vec![SharedHealthCheckManager::LOCK_KEY.to_owned()]
    );
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
}

#[tokio::test]
async fn a_lock_held_by_another_pod_is_left_alone() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_7");
    let manager = world.manager();
    manager.release_health_check_lock().await;
    assert!(world.cache.deletes().is_empty());
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
}

#[tokio::test]
async fn a_missing_lock_deletes_nothing() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    manager.release_health_check_lock().await;
    assert!(world.cache.deletes().is_empty());
    assert_eq!(
        world.ops.entries(),
        vec![format!("get:{}", SharedHealthCheckManager::LOCK_KEY)]
    );
}

#[tokio::test]
async fn a_non_text_owner_is_never_ours() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_json(SharedHealthCheckManager::LOCK_KEY, "42");
    let manager = world.manager();
    manager.release_health_check_lock().await;
    assert!(world.cache.deletes().is_empty());
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
}

#[tokio::test]
async fn a_failed_read_never_deletes() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Get, SharedHealthCheckManager::LOCK_KEY);
    let manager = world.manager();
    manager.release_health_check_lock().await;
    assert!(world.cache.deletes().is_empty());
}

#[tokio::test]
async fn a_failed_delete_keeps_the_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    assert!(manager.acquire_health_check_lock().await);
    world
        .cache
        .fail(Fault::Delete, SharedHealthCheckManager::LOCK_KEY);
    manager.release_health_check_lock().await;
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
}

#[tokio::test]
async fn no_redis_releases_nothing() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.local_manager();
    manager.release_health_check_lock().await;
    assert!(world.ops.entries().is_empty());
}
