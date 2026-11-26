#![cfg(test)]

use std::time::Duration;

use crate::health_cycle::endpoints::HealthCheckResult;

use super::error::SharedHealthError;
use super::manager::SharedHealthCheckManager;
use super::support::{
    Fault, RecordingSleeper, SHARED_AT, World, empty_call, instant, probe_result,
};

async fn perform(
    world: &World,
    manager: &SharedHealthCheckManager,
) -> Result<HealthCheckResult, SharedHealthError> {
    let mut sleeper = RecordingSleeper::default();
    manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
}

#[tokio::test]
async fn a_lock_set_error_falls_back_to_a_local_check() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Set, SharedHealthCheckManager::LOCK_KEY);
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the local check runs");
    assert_eq!(result, probe_result());
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(5)]);
    assert_eq!(world.probe.calls().len(), 1);
    assert!(!world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
}

#[tokio::test]
async fn a_cache_read_error_runs_the_check() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Get, SharedHealthCheckManager::CACHE_KEY);
    let manager = world.manager();
    let result = perform(&world, &manager)
        .await
        .expect("the check runs despite the read error");
    assert_eq!(result, probe_result());
    assert!(world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
}

#[tokio::test]
async fn a_cache_write_error_keeps_the_result_and_releases_the_lock() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Set, SharedHealthCheckManager::CACHE_KEY);
    let manager = world.manager();
    let result = perform(&world, &manager)
        .await
        .expect("the probe result survives the write error");
    assert_eq!(result, probe_result());
    assert!(!world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert_eq!(
        world.cache.deletes(),
        vec![SharedHealthCheckManager::LOCK_KEY.to_owned()]
    );
}

#[tokio::test]
async fn a_release_read_error_keeps_the_result_and_the_lock() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Get, SharedHealthCheckManager::LOCK_KEY);
    let manager = world.manager();
    let result = perform(&world, &manager)
        .await
        .expect("the probe result survives the release error");
    assert_eq!(result, probe_result());
    assert!(world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert!(world.cache.deletes().is_empty());
}

#[tokio::test]
async fn a_release_delete_error_keeps_the_result_and_the_lock() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .fail(Fault::Delete, SharedHealthCheckManager::LOCK_KEY);
    let manager = world.manager();
    let result = perform(&world, &manager)
        .await
        .expect("the probe result survives the delete error");
    assert_eq!(result, probe_result());
    assert!(world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert!(world.cache.deletes().is_empty());
}
