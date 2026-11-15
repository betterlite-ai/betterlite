use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::{Instant, timeout};

use super::support::{FakeFailure, FakeSink, Reply, capture_logs, health_harness};
use crate::db_lifecycle::health::{HEALTH_CHECK_BUDGET, HealthCheck};

#[tokio::test(start_paused = true)]
async fn probe_failure_retries_three_times_and_reports_each_try() {
    let (logs, guard) = capture_logs();
    let harness = health_harness();
    let started = Instant::now();
    harness.state.script_failing_probes(3, FakeFailure::Dropped);
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    drop(guard);
    assert!(matches!(outcome, Err(FakeFailure::Dropped)));
    assert_eq!(started.elapsed(), Duration::from_secs(3));
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 3);
    let failures = harness.reporter.failures();
    assert_eq!(failures.len(), 3);
    assert!(
        failures
            .iter()
            .all(|failure| failure.message == "the connection dropped"),
        "{failures:?}"
    );
    assert!(
        failures
            .iter()
            .all(|failure| failure.duration == Duration::ZERO)
    );
    let lines = logs.lines();
    let warnings = lines
        .iter()
        .filter(|line| {
            line.contains("LiteLLM Prisma Client Exception health_check(): the connection dropped")
        })
        .count();
    assert_eq!(warnings, 3, "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Backing off... this was attempt #1"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Backing off... this was attempt #2"))
    );
}

#[tokio::test(start_paused = true)]
async fn probe_success_stops_the_retries() {
    let harness = health_harness();
    harness
        .state
        .script_probe(Reply::Value(Err(FakeFailure::Dropped)));
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    assert!(outcome.is_ok());
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.reporter.failures().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn planned_replacement_after_a_moved_generation_is_not_reported() {
    let (logs, guard) = capture_logs();
    let harness = health_harness();
    harness.state.script_probe(Reply::After(
        HEALTH_CHECK_BUDGET.saturating_add(Duration::from_secs(1)),
        Err(FakeFailure::Dropped),
    ));
    *harness.state.generation_during_probe.lock().unwrap() = Some(1);
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    drop(guard);
    assert!(matches!(outcome, Err(FakeFailure::Dropped)));
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 2);
    assert!(harness.reporter.failures().is_empty());
    assert_eq!(
        *harness.state.replacement_settles.lock().unwrap(),
        [Duration::from_secs(5)]
    );
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Prisma health_check() connection error raced a planned query-engine replacement; \
             not reporting it as a DB exception: the connection dropped"
        )),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(start_paused = true)]
async fn planned_replacement_without_a_moved_generation_is_reported() {
    let harness = health_harness();
    harness.state.script_probe(Reply::After(
        HEALTH_CHECK_BUDGET.saturating_add(Duration::from_secs(1)),
        Err(FakeFailure::Dropped),
    ));
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    assert!(matches!(outcome, Err(FakeFailure::Dropped)));
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *harness.state.replacement_settles.lock().unwrap(),
        [Duration::from_secs(5)]
    );
    assert_eq!(harness.reporter.failures().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn planned_replacement_needs_a_fresh_probe_to_answer() {
    let (logs, guard) = capture_logs();
    let harness = health_harness();
    harness.state.script_probe(Reply::After(
        HEALTH_CHECK_BUDGET.saturating_add(Duration::from_secs(1)),
        Err(FakeFailure::Dropped),
    ));
    harness
        .state
        .script_probe(Reply::Value(Err(FakeFailure::Caller)));
    *harness.state.generation_during_probe.lock().unwrap() = Some(2);
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    drop(guard);
    assert!(matches!(outcome, Err(FakeFailure::Dropped)));
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.reporter.failures().len(), 1);
    assert!(
        logs.lines().iter().any(|line| line.contains(
            "Prisma health_check() confirmation probe failed: the caller sent a bad key"
        )),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(start_paused = true)]
async fn a_caller_error_is_never_absorbed_by_a_replacement() {
    let harness = health_harness();
    harness.state.script_probe(Reply::After(
        HEALTH_CHECK_BUDGET.saturating_add(Duration::from_secs(1)),
        Err(FakeFailure::Caller),
    ));
    *harness.state.generation_during_probe.lock().unwrap() = Some(3);
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    assert!(matches!(outcome, Err(FakeFailure::Caller)));
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
    assert!(harness.state.replacement_settles.lock().unwrap().is_empty());
    assert_eq!(harness.reporter.failures().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn report_is_spawned_and_not_awaited_by_the_caller() {
    let harness = health_harness();
    let gate = Arc::new(Notify::new());
    *harness.reporter.gate.lock().unwrap() = Some(Arc::clone(&gate));
    harness.state.script_failing_probes(3, FakeFailure::Dropped);
    let outcome = timeout(Duration::from_secs(30), harness.checker.run_health_check()).await;
    tokio::task::yield_now().await;
    assert!(
        matches!(outcome, Ok(Err(FakeFailure::Dropped))),
        "{outcome:?}"
    );
    assert_eq!(harness.reporter.started.load(Ordering::SeqCst), 3);
    assert!(harness.reporter.failures().is_empty());
    gate.notify_waiters();
    harness.checker.tasks.wait().await;
    assert_eq!(harness.reporter.failures().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn health_check_gives_up_after_the_budget() {
    let harness = health_harness();
    harness.state.script_probe(Reply::After(
        Duration::from_secs(10),
        Err(FakeFailure::Dropped),
    ));
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    assert!(matches!(outcome, Err(FakeFailure::Dropped)));
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
    let failures = harness.reporter.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].duration, Duration::from_secs(10));
}

#[tokio::test(start_paused = true)]
async fn health_check_retries_just_under_the_budget() {
    let harness = health_harness();
    harness.state.script_probe(Reply::After(
        Duration::from_secs(9),
        Err(FakeFailure::Dropped),
    ));
    let outcome = harness.checker.run_health_check().await;
    harness.checker.tasks.wait().await;
    assert!(outcome.is_ok());
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_health_check_trait_method_runs_the_probe() {
    let harness = health_harness();
    let outcome = harness.checker.health_check().await;
    assert!(outcome.is_ok());
    assert_eq!(harness.state.probe_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn spend_logs_row_count_reads_the_estimate() {
    let harness = health_harness();
    harness.state.script_row_count(Reply::Value(Ok(Some(42))));
    assert_eq!(harness.checker.spend_logs_row_count().await, 42);
    assert_eq!(harness.state.row_count_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn spend_logs_row_count_retries_until_it_answers() {
    let harness = health_harness();
    harness
        .state
        .script_row_count(Reply::Value(Err(FakeFailure::Dropped)));
    harness.state.script_row_count(Reply::Value(Ok(Some(5))));
    assert_eq!(harness.checker.spend_logs_row_count().await, 5);
    assert_eq!(harness.state.row_count_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn spend_logs_row_count_returns_zero_after_three_failures() {
    let (logs, guard) = capture_logs();
    let harness = health_harness();
    harness
        .state
        .script_failing_row_counts(3, FakeFailure::Dropped);
    let row_count = harness.checker.spend_logs_row_count().await;
    drop(guard);
    assert_eq!(row_count, 0);
    assert_eq!(harness.state.row_count_calls.load(Ordering::SeqCst), 3);
    assert!(
        logs.lines().iter().any(|line| line
            .contains("Error getting LiteLLM_SpendLogs row count: the connection dropped")),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(start_paused = true)]
async fn spend_logs_row_count_treats_a_missing_estimate_as_a_failure() {
    let (logs, guard) = capture_logs();
    let harness = health_harness();
    harness.state.script_missing_estimates(3);
    let row_count = harness.checker.spend_logs_row_count().await;
    drop(guard);
    assert_eq!(row_count, 0);
    assert_eq!(harness.state.row_count_calls.load(Ordering::SeqCst), 3);
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.contains("no reltuples estimate")),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(start_paused = true)]
async fn spend_logs_row_count_gives_up_after_the_budget() {
    let harness = health_harness();
    harness.state.script_row_count(Reply::After(
        Duration::from_secs(11),
        Err(FakeFailure::Dropped),
    ));
    assert_eq!(harness.checker.spend_logs_row_count().await, 0);
    assert_eq!(harness.state.row_count_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn publish_stores_the_row_count_in_the_sink() {
    let harness = health_harness();
    harness.state.script_row_count(Reply::Value(Ok(Some(9))));
    let sink = FakeSink::default();
    harness.checker.publish_spend_logs_row_count(&sink).await;
    assert_eq!(*sink.row_counts.lock().unwrap(), [9]);
}
