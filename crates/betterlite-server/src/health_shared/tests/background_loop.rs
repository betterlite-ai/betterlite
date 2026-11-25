#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_db::queues::pod_lock::PodLockManager;

use crate::health_status_routes::state::HealthResultsSource;

use super::background::{BACKGROUND_CHECKED_BY, SAVE_JOB_ID, background_loop, window_gated_save};
use super::error::SharedHealthError;
use super::manager::SharedHealthCheckManager;
use super::support::{
    FakeLock, POD_A, POD_B, ProbeCall, SHARED_AT, SaveOutcome, StepSleeper, World, deployment,
    instant, run_until_sleep,
};

#[tokio::test]
async fn a_zero_interval_never_runs_a_cycle() {
    let world = World::new(instant(SHARED_AT));
    let mut deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    deps.interval_seconds = 0;
    let outcome = tokio::time::timeout(
        Duration::from_millis(50),
        background_loop(deps, StepSleeper::default()),
    )
    .await;
    outcome.expect("a zero interval returns without a cycle");
    assert!(world.probe.calls().is_empty());
    assert_eq!(world.store.reads(), 0);
}

#[tokio::test]
async fn a_negative_interval_never_runs_a_cycle() {
    let world = World::new(instant(SHARED_AT));
    let mut deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    deps.interval_seconds = -30;
    let outcome = tokio::time::timeout(
        Duration::from_millis(50),
        background_loop(deps, StepSleeper::default()),
    )
    .await;
    outcome.expect("a negative interval returns without a cycle");
    assert!(world.probe.calls().is_empty());
    assert_eq!(world.store.reads(), 0);
}

#[tokio::test]
async fn a_cycle_probes_saves_and_keeps_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    let deployments = vec![
        deployment("probe-ok", "probe-ok", false),
        deployment("probe-down", "probe-down", false),
    ];
    let deps = world.background(None, deployments);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(300)]);
    assert_eq!(
        world.probe.calls(),
        vec![ProbeCall {
            model_count: 2,
            details: true,
            max_concurrency: None,
            skip_disabled: true,
            cycle_id: Some("bg-1".to_owned()),
        }]
    );
    assert_eq!(world.store.reads(), 1);
    let rows = world.store.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].model_id.as_deref(), Some("probe-ok"));
    assert_eq!(rows[0].status, "healthy");
    assert_eq!(rows[0].checked_by.as_deref(), Some(BACKGROUND_CHECKED_BY));
    assert_eq!(rows[1].model_id.as_deref(), Some("probe-down"));
    assert_eq!(rows[1].status, "unhealthy");
    assert_eq!(rows[1].error_message.as_deref(), Some("probe-down is down"));
    assert!(world.lock.holds(SAVE_JOB_ID));
    assert!(!world.lock.released(SAVE_JOB_ID));
    let snapshot = world.results.snapshot();
    assert_eq!(snapshot.healthy_count, 1);
    assert_eq!(snapshot.unhealthy_count, 1);
    let writes = world.writer.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].exceptions.len(), 1);
    assert_eq!(writes[0].healthy.len(), 1);
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls().len(), 2);
    assert_eq!(world.store.reads(), 1);
    assert!(world.lock.holds(SAVE_JOB_ID));
}

#[tokio::test]
async fn the_shared_manager_names_the_pod_and_caches_the_result() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.manager();
    let deps = world.background(
        Some(Arc::clone(&manager)),
        vec![deployment("probe-ok", "probe-ok", false)],
    );
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    let rows = world.store.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model_id.as_deref(), Some("probe-ok"));
    assert_eq!(rows[0].checked_by.as_deref(), Some(manager.pod_id.as_str()));
    assert!(world.cache.has(SharedHealthCheckManager::CACHE_KEY));
    assert!(!world.cache.has(SharedHealthCheckManager::LOCK_KEY));
    assert_eq!(world.results.snapshot().healthy_count, 1);
}

#[tokio::test]
async fn a_failed_shared_check_falls_back_to_the_direct_probe() {
    let world = World::new(instant(SHARED_AT));
    world.probe.queue(Err(SharedHealthError::Probe {
        message: "the shared check failed".to_owned(),
    }));
    let deps = world.background(
        Some(world.manager()),
        vec![deployment("probe-ok", "probe-ok", false)],
    );
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls().len(), 2);
    let rows = world.store.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model_id.as_deref(), Some("probe-ok"));
    assert!(!world.cache.has(SharedHealthCheckManager::CACHE_KEY));
}

#[tokio::test]
async fn two_failed_checks_publish_the_default_result() {
    let world = World::new(instant(SHARED_AT));
    world.probe.queue(Err(SharedHealthError::Probe {
        message: "first failure".to_owned(),
    }));
    world.probe.queue(Err(SharedHealthError::Probe {
        message: "second failure".to_owned(),
    }));
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls().len(), 2);
    let snapshot = world.results.snapshot();
    assert!(snapshot.healthy_endpoints.is_empty());
    assert_eq!(snapshot.healthy_count, 0);
    assert_eq!(snapshot.unhealthy_count, 0);
    assert!(world.store.rows().is_empty());
    let writes = world.writer.writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].healthy.is_empty());
    assert!(writes[0].unhealthy.is_empty());
}

#[tokio::test]
async fn disabled_deployments_are_left_out_of_the_cycle() {
    let world = World::new(instant(SHARED_AT));
    let deployments = vec![
        deployment("probe-ok", "probe-ok", false),
        deployment("probe-off", "probe-off", true),
    ];
    let mut deps = world.background(None, deployments);
    deps.details = false;
    deps.max_concurrency = Some(2);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(
        world.probe.calls(),
        vec![ProbeCall {
            model_count: 1,
            details: false,
            max_concurrency: Some(2),
            skip_disabled: true,
            cycle_id: Some("bg-1".to_owned()),
        }]
    );
}

#[tokio::test]
async fn a_missing_deployment_list_probes_nothing() {
    let world = World::new(instant(SHARED_AT));
    let mut deps = world.background(None, Vec::new());
    deps.deployments = None;
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls()[0].model_count, 0);
    assert!(world.store.rows().is_empty());
}

#[tokio::test]
async fn a_completed_save_keeps_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.pod_manager(POD_A);
    let saved = window_gated_save(Some(&manager), &world.tasks, Some(300), async { true }).await;
    assert!(saved);
    world.tasks.close();
    world.tasks.wait().await;
    assert!(world.lock.holds(SAVE_JOB_ID));
    assert!(!world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn a_failed_save_releases_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.pod_manager(POD_A);
    let saved = window_gated_save(Some(&manager), &world.tasks, Some(300), async { false }).await;
    assert!(!saved);
    assert!(!world.lock.holds(SAVE_JOB_ID));
    assert!(world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn another_pods_window_skips_the_save() {
    let world = World::new(instant(SHARED_AT));
    let other = world.pod_manager(POD_B);
    assert_eq!(
        other.acquire_lock(SAVE_JOB_ID, Some(300), false).await,
        Some(true)
    );
    let manager = world.pod_manager(POD_A);
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let saved = window_gated_save(Some(&manager), &world.tasks, Some(300), async move {
        flag.store(true, Ordering::Relaxed);
        true
    })
    .await;
    assert!(!saved);
    assert!(!ran.load(Ordering::Relaxed));
    assert!(world.lock.holds(SAVE_JOB_ID));
    assert!(!world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn a_pod_lock_without_a_cache_saves_ungated() {
    let world = World::new(instant(SHARED_AT));
    let manager = world.bare_pod_manager();
    let first = window_gated_save(Some(&manager), &world.tasks, Some(300), async { true }).await;
    let second = window_gated_save(Some(&manager), &world.tasks, Some(300), async { false }).await;
    assert!(first);
    assert!(!second);
    assert!(!world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn no_pod_lock_saves_ungated() {
    let world = World::new(instant(SHARED_AT));
    let lock: Option<&Arc<PodLockManager<FakeLock>>> = None;
    let saved = window_gated_save(lock, &world.tasks, None, async { true }).await;
    assert!(saved);
}

#[tokio::test]
async fn a_failed_database_save_releases_the_window_lock_and_retries() {
    let world = World::new(instant(SHARED_AT));
    world.store.behave(SaveOutcome::Fail);
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert!(!world.lock.holds(SAVE_JOB_ID));
    assert!(world.lock.released(SAVE_JOB_ID));
    assert_eq!(world.store.reads(), 1);
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.store.reads(), 2);
}

#[tokio::test]
async fn a_cancelled_save_releases_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    world.store.behave(SaveOutcome::Hang);
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper));
    tokio::select! {
        () = world.store.saved_once() => {}
        () = &mut loop_future => panic!("the background loop returned"),
    }
    assert!(world.lock.holds(SAVE_JOB_ID));
    drop(loop_future);
    world.tasks.close();
    tokio::time::timeout(Duration::from_secs(5), world.tasks.wait())
        .await
        .expect("the release task finishes");
    assert!(world.lock.released(SAVE_JOB_ID));
    assert!(!world.lock.holds(SAVE_JOB_ID));
}
