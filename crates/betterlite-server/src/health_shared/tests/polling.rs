#![cfg(test)]

use std::time::Duration;

use super::manager::SharedHealthCheckManager;
use super::settings::SharedHealthSettings;
use super::support::{
    Fault, RecordingSleeper, SHARED_AT, SHARED_SECONDS, World, cached_text, empty_call,
    healthy_row, instant, probe_result,
};

#[tokio::test]
async fn a_waiting_pod_takes_the_first_fresh_entry() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    world.cache.hide_for(SharedHealthCheckManager::CACHE_KEY, 2);
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_9");
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the entry another pod cached is returned");
    assert_eq!(result.healthy_endpoints, vec![healthy_row("cached-ok")]);
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(5); 2]);
    assert!(world.probe.calls().is_empty());
    let keys: Vec<String> = world
        .cache
        .sets()
        .into_iter()
        .map(|call| call.key)
        .collect();
    assert_eq!(keys, vec![SharedHealthCheckManager::LOCK_KEY.to_owned()]);
    assert!(world.cache.deletes().is_empty());
}

#[tokio::test]
async fn an_empty_cache_waits_the_lock_ttl_then_runs_locally() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_9");
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the local check runs after the wait");
    assert_eq!(result, probe_result());
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(5); 12]);
    assert_eq!(world.probe.calls().len(), 1);
    assert!(world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert!(world.cache.deletes().is_empty());
}

#[tokio::test]
async fn the_wait_is_capped_by_the_configured_lock_ttl() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_9");
    let settings = SharedHealthSettings {
        health_check_ttl: 300,
        lock_ttl: 12,
    };
    let manager = world.manager_with(settings);
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the local check runs after the shorter wait");
    assert_eq!(result, probe_result());
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(5); 3]);
}

#[tokio::test]
async fn a_released_lock_ends_the_wait_without_a_cache() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_9");
    world.cache.hide_for(SharedHealthCheckManager::LOCK_KEY, 1);
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the local check runs at once");
    assert_eq!(result, probe_result());
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(5)]);
    assert_eq!(world.probe.calls().len(), 1);
}

#[tokio::test]
async fn a_failed_lock_read_keeps_waiting() {
    let world = World::new(instant(SHARED_AT));
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_9");
    world
        .cache
        .fail(Fault::Get, SharedHealthCheckManager::LOCK_KEY);
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the local check runs after the full wait");
    assert_eq!(result, probe_result());
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(5); 12]);
    assert_eq!(world.probe.calls().len(), 1);
}
