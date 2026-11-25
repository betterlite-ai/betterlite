#![cfg(test)]

use std::pin::pin;
use std::time::Duration;

use futures_util::future::{Either, select};
use tokio::time::timeout;

use super::manager::SharedHealthCheckManager;
use super::support::{Fault, RecordingSleeper, SHARED_AT, World, empty_call, instant};

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

#[tokio::test]
async fn dropping_the_check_future_while_it_probes_releases_the_lock() {
    let world = World::new(instant(SHARED_AT));
    world.probe.hang();
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let mut check = Box::pin(manager.perform_shared_health_check(
        empty_call(),
        world.probe.as_ref(),
        &mut sleeper,
    ));
    let entered = pin!(world.probe.probe_entered());
    let started = {
        let outcome = select(entered, &mut check).await;
        matches!(outcome, Either::Left(_))
    };
    assert!(started, "the probe starts");
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    drop(check);
    world.tasks.close();
    timeout(Duration::from_secs(5), world.tasks.wait())
        .await
        .expect("the release task finishes");
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert_eq!(
        world.cache.deletes(),
        vec![SharedHealthCheckManager::LOCK_KEY.to_owned()]
    );
}

#[tokio::test]
async fn dropping_the_check_future_while_it_caches_releases_the_lock() {
    let world = World::new(instant(SHARED_AT));
    world.cache.hang_on_set(SharedHealthCheckManager::CACHE_KEY);
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let mut check = Box::pin(manager.perform_shared_health_check(
        empty_call(),
        world.probe.as_ref(),
        &mut sleeper,
    ));
    let entered = pin!(world.cache.write_entered());
    let started = {
        let outcome = select(entered, &mut check).await;
        matches!(outcome, Either::Left(_))
    };
    assert!(started, "the cache write starts");
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    drop(check);
    world.tasks.close();
    timeout(Duration::from_secs(5), world.tasks.wait())
        .await
        .expect("the release task finishes");
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert_eq!(
        world.cache.deletes(),
        vec![SharedHealthCheckManager::LOCK_KEY.to_owned()]
    );
}
