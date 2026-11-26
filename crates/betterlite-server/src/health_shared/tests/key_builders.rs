#![cfg(test)]

use super::manager::SharedHealthCheckManager;
use super::support::{SHARED_AT, World, healthy_row, instant};

#[test]
fn the_lock_key_is_the_sources_lock_name() {
    assert_eq!(SharedHealthCheckManager::LOCK_KEY, "health_check_lock");
}

#[test]
fn the_cache_key_is_the_sources_results_name() {
    assert_eq!(SharedHealthCheckManager::CACHE_KEY, "health_check_results");
}

#[test]
fn the_two_keys_are_distinct() {
    assert_ne!(
        SharedHealthCheckManager::LOCK_KEY,
        SharedHealthCheckManager::CACHE_KEY
    );
}

#[tokio::test]
async fn the_manager_writes_each_key_where_it_says() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    assert!(manager.acquire_health_check_lock().await);
    manager
        .cache_health_check_results(&[healthy_row("probe-ok")], &[])
        .await;
    let keys: Vec<String> = world
        .cache
        .sets()
        .into_iter()
        .map(|call| call.key)
        .collect();
    assert_eq!(
        keys,
        vec![
            SharedHealthCheckManager::LOCK_KEY.to_owned(),
            SharedHealthCheckManager::CACHE_KEY.to_owned(),
        ]
    );
}
