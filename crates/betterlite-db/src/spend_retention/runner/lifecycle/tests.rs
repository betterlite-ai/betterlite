#![cfg(test)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use betterlite_prometheus::collector_registry::CollectorRegistry;
use rstest::rstest;
use tokio::time::Instant;
use tokio_postgres::Client;

use crate::cleanup_metrics::{CleanupMetrics, RunOutcome};
use crate::postgres_store::session::PostgresStore;
use crate::spend_retention::batches::deadline::MonotonicClock;
use crate::spend_retention::batches::settings::CleanupSettings;
use crate::spend_retention::batches::table::CleanupTable;
use crate::spend_retention::batches::{SpendLogCleanup, StopReason, TableCleanupResult};
use crate::spend_retention::runner::clock::{CleanupClock, SystemCleanupClock};
use crate::spend_retention::runner::error::{LockReleaseError, RunError};
use crate::spend_retention::runner::job::SpendLogCleanupJob;
use crate::spend_retention::runner::support::{
    FakeLock, FakePartitions, FixedCleanupClock, HEALTH_IDS, SESSION_COUNT, SESSION_IDS,
    SPEND_COUNT, SPEND_IDS, SlowLock, StepCleanupClock, TAG_IDS, TestDurations, count_rows,
    install_logs, moment, naive, observer, provision, retention_settings, runs, seed_daily_tag,
    seed_health_check, seed_session, seed_sessions, seed_spend_logs, text_join,
};

use super::{log_run_summary, run_outcome};

const SPEND_30D: &str = r#"{"maximum_spend_logs_retention_period": "30d"}"#;
const SPEND_AND_SESSIONS: &str = r#"{"maximum_spend_logs_retention_period": "30d", "maximum_autorouter_session_retention_period": "30d"}"#;
const HORIZONS: &str = r#"{"maximum_spend_logs_retention_period": "10d", "maximum_autorouter_session_retention_period": "3d", "maximum_health_check_retention_period": "1d", "maximum_daily_tag_spend_retention_period": "8d"}"#;
const PARTITIONED: &str = r#"{"maximum_spend_logs_retention_period": "30d", "maximum_autorouter_session_retention_period": "30d", "use_spend_logs_partitioning": true}"#;

fn fixed_clock() -> FixedCleanupClock {
    FixedCleanupClock {
        utc: moment(2025, 2, 1),
        mono: Instant::now(),
    }
}

fn fixed_cleanup(
    settings: CleanupSettings,
    partitions: FakePartitions,
    lock: Option<FakeLock>,
) -> SpendLogCleanup<FixedCleanupClock, TestDurations, FakePartitions, FakeLock> {
    SpendLogCleanup::new(settings, fixed_clock(), TestDurations, partitions, lock)
}

fn step_cleanup(
    settings: CleanupSettings,
    clock: StepCleanupClock,
) -> SpendLogCleanup<StepCleanupClock, TestDurations, FakePartitions, FakeLock> {
    SpendLogCleanup::new(
        settings,
        clock,
        TestDurations,
        FakePartitions::default(),
        None,
    )
}

fn stopped(reason: StopReason) -> TableCleanupResult {
    TableCleanupResult {
        table: CleanupTable::SpendLogs,
        rows_deleted: 0,
        stop_reason: reason,
    }
}

fn deleted(table: CleanupTable, rows_deleted: u64, stop_reason: StopReason) -> TableCleanupResult {
    TableCleanupResult {
        table,
        rows_deleted,
        stop_reason,
    }
}

fn position(contents: &str, needle: &str) -> usize {
    contents
        .find(needle)
        .unwrap_or_else(|| panic!("missing log line {needle}"))
}

async fn run_until_rows_are_deleted<Run>(mut run: Pin<&mut Run>, observer: &Client, before: i64)
where
    Run: Future,
{
    loop {
        tokio::select! {
            _finished = &mut run => panic!("the cleanup finished before it could be cancelled"),
            () = tokio::time::sleep(Duration::from_millis(5)) => {
                if count_rows(observer, SPEND_COUNT).await < before {
                    return;
                }
            }
        }
    }
}

#[tokio::test]
async fn skips_disabled_before_the_lock_is_ever_taken() {
    let mut case = provision().await;
    let cleanup = fixed_cleanup(
        retention_settings("{}"),
        FakePartitions::default(),
        Some(FakeLock::backed(true)),
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a skipped run");
    assert_eq!(runs(&registry, "skipped_disabled"), Some(1));
    assert_eq!(runs(&registry, "completed"), None);
    let lock = cleanup.pod_lock.as_ref().expect("the lock manager");
    assert!(lock.calls().is_empty());
    case.close().await;
}

#[tokio::test]
async fn skips_locked_when_another_pod_holds_the_lease() {
    let mut case = provision().await;
    case.seed_log("expired-log", naive(2024, 12, 1, 0)).await;
    let cleanup = fixed_cleanup(
        retention_settings(SPEND_30D),
        FakePartitions::default(),
        Some(FakeLock::backed(false)),
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a skipped run");
    assert_eq!(runs(&registry, "skipped_locked"), Some(1));
    assert_eq!(runs(&registry, "completed"), None);
    let lock = cleanup.pod_lock.as_ref().expect("the lock manager");
    assert_eq!(lock.calls(), vec!["acquire"]);
    assert_eq!(text_join(&case.client, SPEND_IDS).await, "expired-log");
    case.close().await;
}

#[tokio::test]
async fn a_manager_without_a_redis_cache_runs_without_a_lease() {
    let mut case = provision().await;
    let cleanup = fixed_cleanup(
        retention_settings(SPEND_30D),
        FakePartitions::default(),
        Some(FakeLock::default()),
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(runs(&registry, "completed"), Some(1));
    let lock = cleanup.pod_lock.as_ref().expect("the lock manager");
    assert!(lock.calls().is_empty());
    case.close().await;
}

#[tokio::test]
async fn a_redis_lease_is_taken_and_released_around_the_run() {
    let mut case = provision().await;
    let cleanup = fixed_cleanup(
        retention_settings(SPEND_30D),
        FakePartitions::default(),
        Some(FakeLock::backed(true)),
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(runs(&registry, "completed"), Some(1));
    let lock = cleanup.pod_lock.as_ref().expect("the lock manager");
    assert_eq!(lock.calls(), vec!["acquire", "release"]);
    case.close().await;
}

#[tokio::test]
async fn a_failed_release_reaches_the_caller_instead_of_a_completed_run() {
    let mut case = provision().await;
    let mut lock = FakeLock::backed(true);
    lock.release_failure = true;
    let cleanup = fixed_cleanup(
        retention_settings(SPEND_30D),
        FakePartitions::default(),
        Some(lock),
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let result = cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await;
    assert!(matches!(result, Err(RunError::Release(LockReleaseError))));
    assert_eq!(runs(&registry, "completed"), Some(1));
    case.close().await;
}

#[tokio::test]
async fn a_partition_failure_aborts_the_run_without_reaching_the_next_group() {
    let mut case = provision().await;
    seed_session(&case, "expired-session", moment(2024, 12, 1)).await;
    let cleanup = fixed_cleanup(
        retention_settings(PARTITIONED),
        FakePartitions::enabled(true),
        None,
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let (logs, _subscriber) = install_logs();
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("the run returns normally");
    assert_eq!(runs(&registry, "aborted"), Some(1));
    assert_eq!(runs(&registry, "completed"), None);
    let contents = logs.contents();
    assert!(contents.contains("Error during spend log cleanup"));
    assert!(contents.contains("PartitionMaintenanceError"));
    assert_eq!(
        text_join(&case.client, SESSION_IDS).await,
        "expired-session"
    );
    case.close().await;
}

#[tokio::test]
async fn an_unrepresentable_cutoff_aborts_the_run() {
    let mut case = provision().await;
    case.seed_log("expired-log", naive(2024, 12, 1, 0)).await;
    let cleanup = fixed_cleanup(
        retention_settings(r#"{"maximum_spend_logs_retention_period": "huge"}"#),
        FakePartitions::default(),
        None,
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("the run returns normally");
    assert_eq!(runs(&registry, "aborted"), Some(1));
    assert_eq!(text_join(&case.client, SPEND_IDS).await, "expired-log");
    case.close().await;
}

#[tokio::test]
async fn every_group_uses_its_own_horizon() {
    let mut case = provision().await;
    case.seed_log("past-log", naive(2025, 1, 10, 0)).await;
    case.seed_log("edge-log", naive(2025, 1, 25, 0)).await;
    seed_session(&case, "edge-session", moment(2025, 1, 25)).await;
    seed_health_check(&case, "edge-check", moment(2025, 1, 25)).await;
    seed_daily_tag(&case, "edge-tag", "2025-01-25").await;
    seed_daily_tag(&case, "past-tag", "2025-01-10").await;
    let cleanup = fixed_cleanup(
        retention_settings(HORIZONS),
        FakePartitions::default(),
        None,
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(runs(&registry, "completed"), Some(1));
    assert_eq!(text_join(&case.client, SPEND_IDS).await, "edge-log");
    assert_eq!(text_join(&case.client, SESSION_IDS).await, "");
    assert_eq!(text_join(&case.client, HEALTH_IDS).await, "");
    assert_eq!(text_join(&case.client, TAG_IDS).await, "edge-tag");
    case.close().await;
}

#[tokio::test]
async fn a_health_check_retention_alone_keeps_the_cleanup_job_running() {
    let mut case = provision().await;
    case.seed_log("old-log", naive(2024, 12, 1, 0)).await;
    seed_health_check(&case, "old-check", moment(2024, 12, 1)).await;
    let cleanup = fixed_cleanup(
        retention_settings(r#"{"maximum_health_check_retention_period": "30d"}"#),
        FakePartitions::default(),
        None,
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(runs(&registry, "completed"), Some(1));
    assert_eq!(text_join(&case.client, HEALTH_IDS).await, "");
    assert_eq!(text_join(&case.client, SPEND_IDS).await, "old-log");
    case.close().await;
}

#[tokio::test]
async fn a_daily_tag_retention_alone_keeps_the_cleanup_job_running() {
    let mut case = provision().await;
    case.seed_log("old-log", naive(2024, 12, 1, 0)).await;
    seed_daily_tag(&case, "old-day", "2024-12-01").await;
    seed_daily_tag(&case, "horizon", "2025-01-02").await;
    let cleanup = fixed_cleanup(
        retention_settings(r#"{"maximum_daily_tag_spend_retention_period": "30d"}"#),
        FakePartitions::default(),
        None,
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(runs(&registry, "completed"), Some(1));
    assert_eq!(text_join(&case.client, TAG_IDS).await, "horizon");
    assert_eq!(text_join(&case.client, SPEND_IDS).await, "old-log");
    case.close().await;
}

#[tokio::test]
async fn a_completed_run_logs_the_groups_in_order_at_info() {
    let mut case = provision().await;
    let cleanup = fixed_cleanup(
        retention_settings(HORIZONS),
        FakePartitions::default(),
        None,
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let (logs, _subscriber) = install_logs();
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(runs(&registry, "completed"), Some(1));
    let contents = logs.contents();
    let order = [
        "Removing logs older than",
        "auto-router session rollup rows",
        "health-check rows",
        "daily tag spend rows",
    ]
    .map(|needle| position(&contents, needle));
    assert!(order.as_slice().windows(2).all(|pair| pair[0] < pair[1]));
    assert!(contents.contains("outcome=completed"));
    assert!(!contents.contains("WARN"));
    case.close().await;
}

#[tokio::test]
async fn a_spent_budget_is_recorded_and_warned_about() {
    let mut case = provision().await;
    let start = Instant::now();
    let clock = StepCleanupClock {
        utc: moment(2025, 2, 1),
        start,
        step: Duration::from_secs(1_000),
        readings: AtomicU32::new(0),
    };
    let cleanup = step_cleanup(retention_settings(SPEND_30D), clock);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let (logs, _subscriber) = install_logs();
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("the run returns normally");
    assert_eq!(runs(&registry, "budget_exhausted"), Some(1));
    assert_eq!(runs(&registry, "completed"), None);
    let contents = logs.contents();
    assert!(contents.contains("outcome=budget_exhausted"));
    assert!(contents.contains("WARN"));
    case.close().await;
}

#[tokio::test]
async fn a_reached_batch_cap_is_recorded_and_warned_about() {
    let mut case = provision().await;
    seed_spend_logs(&case, naive(2025, 1, 1, 0), 3).await;
    let mut settings = retention_settings(SPEND_30D);
    settings.defaults.max_batches = 1;
    let cleanup = fixed_cleanup(settings, FakePartitions::default(), None);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let (logs, _subscriber) = install_logs();
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("the run returns normally");
    assert_eq!(runs(&registry, "batch_cap_reached"), Some(1));
    let contents = logs.contents();
    assert!(contents.contains("outcome=batch_cap_reached"));
    assert!(contents.contains("WARN"));
    case.close().await;
}

#[test]
fn the_most_significant_stop_reason_wins() {
    assert_eq!(
        run_outcome(&[stopped(StopReason::Exhausted)]),
        RunOutcome::Completed
    );
    assert_eq!(
        run_outcome(&[
            stopped(StopReason::Exhausted),
            stopped(StopReason::BatchCapReached),
        ]),
        RunOutcome::BatchCapReached
    );
    assert_eq!(
        run_outcome(&[
            stopped(StopReason::BatchCapReached),
            stopped(StopReason::BudgetExhausted),
        ]),
        RunOutcome::BudgetExhausted
    );
    assert_eq!(
        run_outcome(&[
            stopped(StopReason::BudgetExhausted),
            stopped(StopReason::Aborted),
        ]),
        RunOutcome::Aborted
    );
    assert_eq!(run_outcome(&[]), RunOutcome::Completed);
}

#[test]
fn the_summary_names_every_table_its_count_and_how_it_stopped() {
    let results = [
        deleted(CleanupTable::SpendLogs, 3, StopReason::Exhausted),
        deleted(
            CleanupTable::AutoRouterSessions,
            0,
            StopReason::BudgetExhausted,
        ),
    ];
    let (logs, _subscriber) = install_logs();
    log_run_summary(
        RunOutcome::BudgetExhausted,
        &results,
        Duration::from_millis(1_500),
    );
    let contents = logs.contents();
    assert!(contents.contains("outcome=budget_exhausted"));
    assert!(contents.contains("elapsed=1.5s"));
    assert!(contents.contains("LiteLLM_SpendLogs: deleted=3 stop_reason=exhausted"));
    assert!(contents.contains("LiteLLM_AutoRouterSession: deleted=0 stop_reason=budget_exhausted"));
    assert!(contents.contains("WARN"));
}

#[test]
fn a_completed_summary_is_logged_at_info() {
    let results = [deleted(
        CleanupTable::HealthChecks,
        0,
        StopReason::Exhausted,
    )];
    let (logs, _subscriber) = install_logs();
    log_run_summary(RunOutcome::Completed, &results, Duration::from_millis(20));
    let contents = logs.contents();
    assert!(contents.contains("INFO"));
    assert!(contents.contains("outcome=completed"));
    assert!(!contents.contains("WARN"));
}

#[rstest]
#[case::one_group(
    300_000,
    r#"{"maximum_spend_logs_retention_period": "30d", "use_spend_logs_partitioning": true}"#
)]
#[case::two_groups(150_000, r#"{"maximum_spend_logs_retention_period": "30d", "maximum_autorouter_session_retention_period": "30d", "use_spend_logs_partitioning": true}"#)]
#[case::four_groups(75_000, r#"{"maximum_spend_logs_retention_period": "30d", "maximum_autorouter_session_retention_period": "30d", "maximum_health_check_retention_period": "30d", "maximum_daily_tag_spend_retention_period": "30d", "use_spend_logs_partitioning": true}"#)]
#[tokio::test]
async fn each_group_takes_an_equal_share_of_the_run_budget(
    #[case] expected: u64,
    #[case] json: &str,
) {
    let mut case = provision().await;
    let cleanup = fixed_cleanup(
        retention_settings(json),
        FakePartitions::enabled(false),
        None,
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(cleanup.partitions.seen_remaining(), vec![expected]);
    assert_eq!(runs(&registry, "completed"), Some(1));
    case.close().await;
}

#[tokio::test]
async fn a_later_group_gets_the_time_the_earlier_groups_left() {
    let mut case = provision().await;
    seed_sessions(&case, moment(2024, 12, 1), 200).await;
    let start = Instant::now();
    let clock = StepCleanupClock {
        utc: moment(2025, 2, 1),
        start,
        step: Duration::from_secs(2),
        readings: AtomicU32::new(0),
    };
    let cleanup = step_cleanup(retention_settings(SPEND_AND_SESSIONS), clock);
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("the run returns normally");
    assert_eq!(runs(&registry, "budget_exhausted"), Some(1));
    assert!(count_rows(&case.client, SESSION_COUNT).await < 166);
    case.close().await;
}

#[tokio::test]
async fn the_budget_starts_after_the_lease_is_acquired() {
    let mut case = provision().await;
    let start = Instant::now();
    let clock = Arc::new(StepCleanupClock {
        utc: moment(2025, 2, 1),
        start,
        step: Duration::from_secs(10),
        readings: AtomicU32::new(0),
    });
    let lock = SlowLock {
        clock: Arc::clone(&clock),
        readings: 50,
    };
    let cleanup = SpendLogCleanup::new(
        retention_settings(SPEND_30D),
        clock,
        TestDurations,
        FakePartitions::default(),
        Some(lock),
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    cleanup
        .cleanup_old_spend_logs(&mut case.client, &metrics)
        .await
        .expect("a completed run");
    assert_eq!(runs(&registry, "completed"), Some(1));
    assert_eq!(runs(&registry, "budget_exhausted"), None);
    case.close().await;
}

#[test]
fn the_system_clock_reads_the_wall_and_monotonic_clocks() {
    let clock = SystemCleanupClock;
    let before = Instant::now();
    assert!(clock.utc_now() > moment(2024, 1, 1));
    assert!(clock.now() >= before);
    assert!(clock.now() <= Instant::now());
}

#[tokio::test]
async fn the_job_handle_reports_a_session_it_cannot_open() {
    let cleanup = Arc::new(fixed_cleanup(
        retention_settings(SPEND_30D),
        FakePartitions::default(),
        None,
    ));
    let store = PostgresStore { pool: None };
    let job = SpendLogCleanupJob::new(cleanup, store, Arc::new(CollectorRegistry::default()));
    let result = job.run().await;
    assert!(matches!(result, Err(RunError::Session { .. })));
}

#[tokio::test]
async fn cancellation_records_aborted_with_the_progress_so_far() {
    let mut case = provision().await;
    seed_spend_logs(&case, naive(2025, 1, 1, 0), 1_000).await;
    let (observer, _tasks) = observer(&case).await;
    let cleanup = fixed_cleanup(
        retention_settings(SPEND_30D),
        FakePartitions::default(),
        Some(FakeLock::backed(true)),
    );
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let (logs, _subscriber) = install_logs();
    let mut future = Box::pin(cleanup.cleanup_old_spend_logs(&mut case.client, &metrics));
    run_until_rows_are_deleted(future.as_mut(), &observer, 1_000).await;
    drop(future);
    assert_eq!(runs(&registry, "aborted"), Some(1));
    assert_eq!(runs(&registry, "completed"), None);
    let remaining = count_rows(&observer, SPEND_COUNT).await;
    let deleted = 1_000 - remaining;
    let contents = logs.contents();
    assert!(contents.contains("Spend log cleanup cancelled after"));
    assert!(contents.contains(&format!("rows_deleted={deleted},")));
    assert!(contents.contains(&format!("batches={})", deleted / 2)));
    let lock = cleanup.pod_lock.as_ref().expect("the lock manager");
    assert_eq!(lock.calls(), vec!["acquire"]);
    case.close().await;
}
