#![cfg(test)]

use std::error::Error as _;

use super::manager::SharedHealthCheckManager;
use super::probe::SharedProbeCall;
use super::support::{
    ProbeCall, RecordingSleeper, SHARED_AT, SHARED_SECONDS, World, cached_text, deployment,
    empty_call, healthy_row, instant, probe_error, probe_result,
};

#[tokio::test]
async fn a_fresh_entry_short_circuits_the_check() {
    let world = World::new(instant(SHARED_AT));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    world
        .cache
        .put_plain(SharedHealthCheckManager::LOCK_KEY, "pod_9");
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the cached result is returned");
    assert_eq!(result.healthy_endpoints, vec![healthy_row("cached-ok")]);
    assert!(result.exceptions_by_model_id.is_empty());
    assert!(world.probe.calls().is_empty());
    assert!(world.cache.sets().is_empty());
    assert!(sleeper.delays().is_empty());
}

#[tokio::test]
async fn the_lock_holder_runs_the_check_and_caches_it() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let models = vec![deployment("probe-ok", "probe-ok", false)];
    let call = SharedProbeCall {
        models: &models,
        details: true,
        max_concurrency: Some(3),
        skip_disabled: true,
        instrumentation: None,
    };
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(call, world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the check runs");
    assert_eq!(result, probe_result());
    assert_eq!(
        world.probe.calls(),
        vec![ProbeCall {
            model_count: 1,
            details: true,
            max_concurrency: Some(3),
            skip_disabled: true,
            cycle_id: None,
        }]
    );
    assert_eq!(
        world.ops.entries(),
        vec![
            format!("get:{}", SharedHealthCheckManager::CACHE_KEY),
            format!("set:{}", SharedHealthCheckManager::LOCK_KEY),
            "probe".to_owned(),
            format!("set:{}", SharedHealthCheckManager::CACHE_KEY),
            format!("get:{}", SharedHealthCheckManager::LOCK_KEY),
            format!("delete:{}", SharedHealthCheckManager::LOCK_KEY),
        ]
    );
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert!(world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(sleeper.delays().is_empty());
}

#[tokio::test]
async fn a_failed_check_is_not_cached_and_releases_the_lock() {
    let world = World::new(instant(SHARED_AT));
    world.probe.queue(Err(probe_error()));
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let error = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect_err("the probe failure is returned");
    assert!(
        error.source().is_some(),
        "the cycle failure is kept as the source"
    );
    assert!(!world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert_eq!(world.cache.sets().len(), 1);
}

#[tokio::test]
async fn an_expired_entry_runs_the_check() {
    let world = World::new(instant("2024-01-01T00:05:01Z"));
    world.cache.put_json(
        SharedHealthCheckManager::CACHE_KEY,
        &cached_text(SHARED_SECONDS, "pod_7"),
    );
    let manager = world.manager();
    let mut sleeper = RecordingSleeper::default();
    let result = manager
        .perform_shared_health_check(empty_call(), world.probe.as_ref(), &mut sleeper)
        .await
        .expect("the expired entry is not returned");
    assert_eq!(result, probe_result());
    assert_eq!(world.probe.calls().len(), 1);
    assert!(sleeper.delays().is_empty());
}
