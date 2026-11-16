use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::task::yield_now;
use tokio::time::advance;

use super::support::{
    FakeFailure, RECONNECT_TIMEOUT, Reply, WATCHDOG_INTERVAL, capture_logs, reconnect_call,
    recreate_call, watchdog_harness, watchdog_harness_with, watchdog_settings,
};
use crate::db_lifecycle::read_only::RecreateState;
use crate::db_lifecycle::watchdog::{
    CONNECTION_ERROR_REASON, WRITER_READ_ONLY_REASON, WRITER_UNAVAILABLE_REASON,
};

#[tokio::test(start_paused = true)]
async fn disabled_watchdog_spawns_nothing() {
    let (logs, guard) = capture_logs();
    let mut settings = watchdog_settings();
    settings.enabled = false;
    let harness = watchdog_harness_with(settings);
    harness.watchdog.start(&harness.tasks);
    advance(WATCHDOG_INTERVAL).await;
    drop(guard);
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 0);
    assert!(harness.watchdog.task.handle.lock().unwrap().is_none());
    assert!(
        logs.lines().iter().any(|line| line
            .contains("Prisma DB health watchdog disabled via PRISMA_HEALTH_WATCHDOG_ENABLED")),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn start_logs_the_watchdog_settings() {
    let (logs, guard) = capture_logs();
    let harness = watchdog_harness();
    harness.watchdog.start(&harness.tasks);
    drop(guard);
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Started Prisma DB health watchdog (interval=30s, reconnect_cooldown=15s, \
             probe_timeout=5.0s, reconnect_timeout=30.0s)"
        )),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(start_paused = true)]
async fn loop_probes_after_the_interval() {
    let harness = watchdog_harness();
    harness.watchdog.start(&harness.tasks);
    yield_now().await;
    advance(WATCHDOG_INTERVAL.saturating_sub(Duration::from_millis(1))).await;
    yield_now().await;
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 0);
    advance(Duration::from_millis(1)).await;
    yield_now().await;
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
    harness.watchdog.stop().await;
}

#[tokio::test(start_paused = true)]
async fn start_twice_runs_one_loop() {
    let (logs, guard) = capture_logs();
    let harness = watchdog_harness();
    harness.watchdog.start(&harness.tasks);
    harness.watchdog.start(&harness.tasks);
    yield_now().await;
    advance(WATCHDOG_INTERVAL).await;
    yield_now().await;
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
    advance(WATCHDOG_INTERVAL).await;
    yield_now().await;
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 2);
    harness.watchdog.stop().await;
    drop(guard);
    let started = logs
        .lines()
        .iter()
        .filter(|line| line.contains("Started Prisma DB health watchdog"))
        .count();
    assert_eq!(started, 1, "{:?}", logs.lines());
}

#[tokio::test]
async fn writer_unavailable_reconnects_and_skips_the_read_only_probe() {
    let harness = watchdog_harness();
    harness.state.script_writer_probe(Reply::Value(Ok(true)));
    harness
        .state
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    harness.watchdog.turn().await;
    assert_eq!(
        harness.reconnect.called(),
        [reconnect_call(
            WRITER_UNAVAILABLE_REASON,
            Some(RECONNECT_TIMEOUT)
        )]
    );
    assert_eq!(
        harness.state.writer_read_only_calls.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn read_only_writer_recreates_the_writer() {
    let (logs, guard) = capture_logs();
    let harness = watchdog_harness();
    harness.state.script_writer_probe(Reply::Value(Ok(true)));
    harness.watchdog.turn().await;
    drop(guard);
    assert_eq!(
        harness.reconnect.called(),
        [recreate_call(
            WRITER_READ_ONLY_REASON,
            Some(RECONNECT_TIMEOUT)
        )]
    );
    assert_eq!(harness.read_only.state.lock().unwrap().streak, 1);
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.contains("Writer session is read-only")),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn writable_writer_resets_the_read_only_streak() {
    let harness = watchdog_harness();
    *harness.read_only.state.lock().unwrap() = RecreateState {
        last_recreate: Some(Instant::now()),
        streak: 3,
    };
    harness.watchdog.turn().await;
    assert_eq!(
        *harness.read_only.state.lock().unwrap(),
        RecreateState::default()
    );
    assert!(harness.reconnect.called().is_empty());
}

#[tokio::test]
async fn writer_unavailable_reconnect_keeps_the_read_only_streak() {
    let harness = watchdog_harness();
    *harness.read_only.state.lock().unwrap() = RecreateState {
        last_recreate: Some(Instant::now()),
        streak: 2,
    };
    harness
        .state
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    harness.watchdog.turn().await;
    assert_eq!(harness.read_only.state.lock().unwrap().streak, 2);
    assert_eq!(harness.reconnect.called().len(), 1);
}

#[tokio::test]
async fn read_only_recreate_keeps_the_streak_growing() {
    let harness = watchdog_harness();
    *harness.read_only.state.lock().unwrap() = RecreateState {
        last_recreate: None,
        streak: 2,
    };
    harness.state.script_writer_probe(Reply::Value(Ok(true)));
    harness.watchdog.turn().await;
    assert_eq!(harness.read_only.state.lock().unwrap().streak, 3);
    assert_eq!(harness.reconnect.called().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn routed_probe_timeout_reconnects_with_the_connection_error_reason() {
    let harness = watchdog_harness();
    harness.state.script_probe(Reply::Pends);
    harness.watchdog.turn().await;
    assert_eq!(
        harness.reconnect.called(),
        [reconnect_call(
            CONNECTION_ERROR_REASON,
            Some(RECONNECT_TIMEOUT)
        )]
    );
    assert_eq!(
        harness.state.writer_read_only_calls.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn infrastructure_probe_error_reconnects() {
    let harness = watchdog_harness();
    harness
        .state
        .script_probe(Reply::Value(Err(FakeFailure::Unavailable)));
    harness.watchdog.turn().await;
    assert_eq!(
        harness.reconnect.called(),
        [reconnect_call(
            CONNECTION_ERROR_REASON,
            Some(RECONNECT_TIMEOUT)
        )]
    );
}

#[tokio::test]
async fn caller_probe_error_is_only_logged() {
    let (logs, guard) = capture_logs();
    let harness = watchdog_harness();
    harness
        .state
        .script_probe(Reply::Value(Err(FakeFailure::Caller)));
    harness.watchdog.turn().await;
    drop(guard);
    assert!(harness.reconnect.called().is_empty());
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Prisma DB health watchdog observed non-DB error: the caller sent a bad key"
        )),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn writer_probe_error_reconnects() {
    let harness = watchdog_harness();
    harness
        .state
        .script_writer_probe(Reply::Value(Err(FakeFailure::Dropped)));
    harness.watchdog.turn().await;
    assert_eq!(
        harness.reconnect.called(),
        [reconnect_call(
            CONNECTION_ERROR_REASON,
            Some(RECONNECT_TIMEOUT)
        )]
    );
}

#[tokio::test(start_paused = true)]
async fn writer_probe_timeout_reconnects() {
    let harness = watchdog_harness();
    harness.state.script_writer_probe(Reply::Pends);
    harness.watchdog.turn().await;
    assert_eq!(
        harness.reconnect.called(),
        [reconnect_call(
            CONNECTION_ERROR_REASON,
            Some(RECONNECT_TIMEOUT)
        )]
    );
}

#[tokio::test(start_paused = true)]
async fn stop_cancels_the_loop() {
    let (logs, guard) = capture_logs();
    let harness = watchdog_harness();
    harness.watchdog.start(&harness.tasks);
    yield_now().await;
    advance(WATCHDOG_INTERVAL).await;
    yield_now().await;
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
    harness.watchdog.stop().await;
    advance(WATCHDOG_INTERVAL.saturating_mul(3)).await;
    yield_now().await;
    drop(guard);
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
    assert!(harness.watchdog.task.handle.lock().unwrap().is_none());
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.contains("Stopped Prisma DB health watchdog")),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn stop_without_start_is_silent() {
    let (logs, guard) = capture_logs();
    let harness = watchdog_harness();
    harness.watchdog.stop().await;
    drop(guard);
    assert!(
        logs.lines()
            .iter()
            .all(|line| !line.contains("Stopped Prisma DB health watchdog")),
        "{:?}",
        logs.lines()
    );
}
