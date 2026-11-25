#![cfg(test)]

use std::future::pending;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_db::queues::pod_lock::PodLockManager;
use futures_util::future::{Either, select};
use tokio::time::timeout;

use crate::health_status_routes::state::HealthResultsSource as _;

use super::background::{
    BACKGROUND_CHECKED_BY, SAVE_DEADLINE, SAVE_JOB_ID, background_loop, window_gated_save,
};
use super::fixtures::FakeLock;
use super::manager::SharedHealthCheckManager;
use super::support::{
    POD_A, POD_B, ProbeCall, SHARED_AT, SaveOutcome, StepSleeper, World, deployment, instant,
    probe_error, run_until_sleep,
};

#[tokio::test]
async fn a_zero_interval_never_runs_a_cycle() {
    let world = World::new(instant(SHARED_AT));
    let mut deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    deps.interval_seconds = 0;
    let outcome = timeout(
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
    let outcome = timeout(
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
    assert_eq!(world.lock.acquire_ttls(), vec![300_i64]);
    let snapshot = world.results.snapshot();
    assert_eq!(snapshot.healthy_count, 1);
    assert_eq!(snapshot.unhealthy_count, 1);
    let writes = world.writer.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].exceptions.len(), 1);
    assert_eq!(writes[0].healthy.len(), 1);
}

#[tokio::test]
async fn a_second_cycle_reprobes_without_saving_again() {
    let world = World::new(instant(SHARED_AT));
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls().len(), 1);
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls().len(), 2);
    assert_eq!(world.store.reads(), 1);
    assert!(world.lock.holds(SAVE_JOB_ID));
}

#[tokio::test]
async fn a_closed_tracker_stops_the_loop_after_the_cycle() {
    let world = World::new(instant(SHARED_AT));
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    world.tasks.close();
    sleeper.wakes.notify_one();
    timeout(Duration::from_millis(50), loop_future)
        .await
        .expect("the closed tracker ends the loop");
    assert_eq!(world.probe.calls().len(), 1);
    assert_eq!(world.store.reads(), 1);
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
    world.probe.queue(Err(probe_error()));
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
async fn a_failed_direct_check_keeps_the_published_results() {
    let world = World::new(instant(SHARED_AT));
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.results.snapshot().healthy_count, 1);
    assert_eq!(world.store.reads(), 1);
    assert_eq!(world.writer.writes().len(), 1);
    world.probe.queue(Err(probe_error()));
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls().len(), 2);
    let snapshot = world.results.snapshot();
    assert_eq!(snapshot.healthy_count, 1);
    assert_eq!(snapshot.unhealthy_count, 1);
    assert_eq!(world.store.reads(), 1);
    assert_eq!(world.writer.writes().len(), 1);
}

#[tokio::test]
async fn a_failed_shared_check_and_fallback_keep_the_published_results() {
    let world = World::new(instant(SHARED_AT));
    let deps = world.background(
        Some(world.manager()),
        vec![deployment("probe-ok", "probe-ok", false)],
    );
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.store.reads(), 1);
    world.cache.remove(SharedHealthCheckManager::CACHE_KEY);
    world.probe.queue(Err(probe_error()));
    world.probe.queue(Err(probe_error()));
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls().len(), 3);
    let snapshot = world.results.snapshot();
    assert_eq!(snapshot.healthy_count, 1);
    assert_eq!(snapshot.unhealthy_count, 1);
    assert_eq!(world.store.reads(), 1);
    assert_eq!(world.writer.writes().len(), 1);
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
async fn a_loop_without_a_store_releases_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    let mut deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    deps.store = None;
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert!(world.lock.released(SAVE_JOB_ID));
    assert!(!world.lock.holds(SAVE_JOB_ID));
}

#[tokio::test]
async fn a_completed_save_keeps_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = World::pod_manager(POD_A, Some(Arc::clone(&world.lock)));
    let saved = window_gated_save(
        Some(&manager),
        &world.tasks,
        Some(300),
        async { true },
        SAVE_DEADLINE,
    )
    .await;
    assert!(saved);
    world.tasks.close();
    world.tasks.wait().await;
    assert!(world.lock.holds(SAVE_JOB_ID));
    assert!(!world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn a_failed_save_releases_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = World::pod_manager(POD_A, Some(Arc::clone(&world.lock)));
    let saved = window_gated_save(
        Some(&manager),
        &world.tasks,
        Some(300),
        async { false },
        SAVE_DEADLINE,
    )
    .await;
    assert!(!saved);
    assert!(!world.lock.holds(SAVE_JOB_ID));
    assert!(world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn another_pods_window_skips_the_save() {
    let world = World::new(instant(SHARED_AT));
    let other = World::pod_manager(POD_B, Some(Arc::clone(&world.lock)));
    assert_eq!(
        other.acquire_lock(SAVE_JOB_ID, Some(300), false).await,
        Some(true)
    );
    let manager = World::pod_manager(POD_A, Some(Arc::clone(&world.lock)));
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let saved = window_gated_save(
        Some(&manager),
        &world.tasks,
        Some(300),
        async move {
            flag.store(true, Ordering::Relaxed);
            true
        },
        SAVE_DEADLINE,
    )
    .await;
    assert!(!saved);
    assert!(!ran.load(Ordering::Relaxed));
    assert!(world.lock.holds(SAVE_JOB_ID));
    assert!(!world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn a_pod_lock_without_a_cache_saves_ungated() {
    let world = World::new(instant(SHARED_AT));
    let manager = World::pod_manager(POD_A, None);
    let first = window_gated_save(
        Some(&manager),
        &world.tasks,
        Some(300),
        async { true },
        SAVE_DEADLINE,
    )
    .await;
    let second = window_gated_save(
        Some(&manager),
        &world.tasks,
        Some(300),
        async { false },
        SAVE_DEADLINE,
    )
    .await;
    assert!(first);
    assert!(!second);
    assert!(!world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn no_pod_lock_saves_ungated() {
    let world = World::new(instant(SHARED_AT));
    let lock: Option<&Arc<PodLockManager<FakeLock>>> = None;
    let saved = window_gated_save(lock, &world.tasks, None, async { true }, SAVE_DEADLINE).await;
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
    let saved = pin!(world.store.saved_once());
    let outcome = select(saved, &mut loop_future).await;
    assert!(
        matches!(outcome, Either::Left(_)),
        "the background loop returned"
    );
    assert!(world.lock.holds(SAVE_JOB_ID));
    drop(loop_future);
    world.tasks.close();
    timeout(Duration::from_secs(5), world.tasks.wait())
        .await
        .expect("the release task finishes");
    assert!(world.lock.released(SAVE_JOB_ID));
    assert!(!world.lock.holds(SAVE_JOB_ID));
}

#[tokio::test]
async fn deployments_added_between_cycles_reach_the_next_probe() {
    let world = World::new(instant(SHARED_AT));
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls()[0].model_count, 1);
    world.deployments.set(vec![
        deployment("probe-ok", "probe-ok", false),
        deployment("probe-new", "probe-new", false),
    ]);
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls()[1].model_count, 2);
}

#[tokio::test]
async fn deployments_removed_between_cycles_leave_the_next_probe() {
    let world = World::new(instant(SHARED_AT));
    let deps = world.background(
        None,
        vec![
            deployment("probe-ok", "probe-ok", false),
            deployment("probe-down", "probe-down", false),
        ],
    );
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls()[0].model_count, 2);
    world
        .deployments
        .set(vec![deployment("probe-ok", "probe-ok", false)]);
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls()[1].model_count, 1);
}

#[tokio::test]
async fn a_deployment_disabled_between_cycles_leaves_the_next_probe() {
    let world = World::new(instant(SHARED_AT));
    let deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls()[0].model_count, 1);
    world
        .deployments
        .set(vec![deployment("probe-ok", "probe-ok", true)]);
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.probe.calls()[1].model_count, 0);
}

#[tokio::test]
async fn a_save_past_its_deadline_releases_the_window_lock() {
    let world = World::new(instant(SHARED_AT));
    let manager = World::pod_manager(POD_A, Some(Arc::clone(&world.lock)));
    let saved = window_gated_save(
        Some(&manager),
        &world.tasks,
        Some(300),
        pending::<bool>(),
        Duration::from_millis(20),
    )
    .await;
    assert!(!saved);
    assert!(!world.lock.holds(SAVE_JOB_ID));
    assert!(world.lock.released(SAVE_JOB_ID));
}

#[tokio::test]
async fn an_ungated_save_past_its_deadline_reports_a_failed_save() {
    let world = World::new(instant(SHARED_AT));
    let lock: Option<&Arc<PodLockManager<FakeLock>>> = None;
    let saved = window_gated_save(
        lock,
        &world.tasks,
        None,
        pending::<bool>(),
        Duration::from_millis(20),
    )
    .await;
    assert!(!saved);
}

#[tokio::test]
async fn a_stalled_save_does_not_block_the_next_cycle() {
    let world = World::new(instant(SHARED_AT));
    world.store.behave(SaveOutcome::Hang);
    let mut deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    deps.save_deadline = Duration::from_millis(20);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert!(!world.lock.holds(SAVE_JOB_ID));
    assert!(world.lock.released(SAVE_JOB_ID));
    sleeper.wakes.notify_one();
    run_until_sleep(&sleeper, &mut loop_future).await;
    assert_eq!(world.store.reads(), 2);
}

#[tokio::test]
async fn a_stalled_save_does_not_hold_shutdown_open() {
    let world = World::new(instant(SHARED_AT));
    world.store.behave(SaveOutcome::Hang);
    let mut deps = world.background(None, vec![deployment("probe-ok", "probe-ok", false)]);
    deps.save_deadline = Duration::from_millis(20);
    let sleeper = StepSleeper::default();
    let mut loop_future = Box::pin(background_loop(deps, sleeper.clone()));
    run_until_sleep(&sleeper, &mut loop_future).await;
    world.tasks.close();
    sleeper.wakes.notify_one();
    timeout(Duration::from_secs(1), loop_future)
        .await
        .expect("the closed tracker ends the loop");
}
