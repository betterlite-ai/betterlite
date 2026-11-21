#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_postgres::{Client, NoTls};

use super::super::cutoff::Cutoff;
use super::super::deadline::MonotonicClock;
use super::super::settings::CleanupBounds;
use super::super::table::CleanupTable;
use super::super::{BatchProgress, StopReason, TableCleanupResult};
use super::BatchDeleter;
use crate::cleanup_metrics::CleanupMetrics;
use crate::spend_partitions::fixtures::SpendPartitionCase;

const ROWS_DELETED: &str = "litellm_spend_log_cleanup_rows_deleted_total";
const BATCH_DURATION: &str = "litellm_spend_log_cleanup_batch_duration_seconds";
const ROWS_REMAINING: &str = "litellm_spend_log_cleanup_rows_remaining";
const BATCH_FAILURES: &str = "litellm_spend_log_cleanup_batch_failures_total";
const SPEND_LOGS: &str = "LiteLLM_SpendLogs";

const BOUNDS: CleanupBounds = CleanupBounds {
    batch_size: 2,
    max_batches: 500,
    run_budget: Duration::from_secs(300),
    batch_timeout: Duration::from_secs(30),
    max_consecutive_failures: 3,
    failure_backoff: Duration::ZERO,
    remaining_count_cap: 100_000,
};

const DAILY_TAGS_TABLE: &str = r#"CREATE TABLE "LiteLLM_DailyTagSpend" ("id" TEXT PRIMARY KEY, "date" TEXT NOT NULL);
INSERT INTO "LiteLLM_DailyTagSpend" VALUES ('old', '2026-01-01'), ('horizon', '2026-01-02'), ('new', '2026-01-03');"#;

// sequence values survive a rolled-back batch, so every other DELETE statement fails
const ALTERNATING_FAILURES: &str = r#"CREATE SEQUENCE cleanup_attempts;
CREATE FUNCTION fail_odd_attempts() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF nextval('cleanup_attempts') % 2 = 1 THEN
        RAISE EXCEPTION 'injected batch failure';
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER fail_odd_attempts BEFORE DELETE ON "LiteLLM_SpendLogs" FOR EACH STATEMENT EXECUTE FUNCTION fail_odd_attempts();"#;

// each reading moves the clock one step, so a test places the deadline between two known readings
struct StepClock {
    start: Instant,
    step: Duration,
    readings: AtomicU32,
}

impl StepClock {
    fn new(step: Duration) -> Self {
        Self {
            start: Instant::now(),
            step,
            readings: AtomicU32::new(0),
        }
    }

    fn at(&self, steps: u32) -> Instant {
        self.start
            .checked_add(self.step.checked_mul(steps).unwrap())
            .unwrap()
    }
}

impl MonotonicClock for StepClock {
    fn now(&self) -> Instant {
        let reading = self.readings.fetch_add(1, Ordering::SeqCst);
        self.at(reading)
    }
}

#[derive(Default)]
struct Batches(Vec<u64>);

impl BatchProgress for Batches {
    fn record_batch(&mut self, rows_deleted: u64) {
        self.0.push(rows_deleted);
    }
}

fn old() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2020, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
}

fn recent() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2030, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
}

fn cutoff() -> Cutoff {
    Cutoff::Timestamp("2025-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap())
}

async fn seeded(expired: u32, kept: u32) -> SpendPartitionCase {
    let case = SpendPartitionCase::provision().await;
    for index in 0..expired {
        case.seed_log(&format!("old-{index}"), old()).await;
    }
    for index in 0..kept {
        case.seed_log(&format!("new-{index}"), recent()).await;
    }
    case
}

async fn row_count(client: &Client, table: &str) -> i64 {
    client
        .query_one(&format!("SELECT count(*) FROM \"{table}\""), &[])
        .await
        .unwrap()
        .get::<_, i64>(0)
}

fn series_total(registry: &CollectorRegistry, name: &str) -> Option<MetricValue> {
    registry
        .get(name)
        .and_then(|metric| metric.series(&[Some(SPEND_LOGS.to_owned())]))
        .map(|sample| sample.total)
}

fn finite(value: i64) -> MetricValue {
    MetricValue::Finite(Decimal::from(value))
}

async fn run_table(
    case: &mut SpendPartitionCase,
    run: (CleanupBounds, &StepClock, &CleanupMetrics<'_>),
    table: CleanupTable,
    target: (Cutoff, Instant),
) -> (TableCleanupResult, Vec<u64>) {
    let (bounds, clock, metrics) = run;
    let mut progress = Batches::default();
    let mut deleter = BatchDeleter {
        client: &mut case.client,
        bounds,
        clock,
        metrics,
        progress: &mut progress,
    };
    let result = deleter.delete_expired(table, target.0, target.1).await;
    (result, progress.0)
}

async fn lock_spend_logs(url: &str, drivers: &mut JoinSet<()>) -> Client {
    let (holder, connection) = tokio_postgres::connect(url, NoTls).await.unwrap();
    drivers.spawn(async move { connection.await.unwrap() });
    holder
        .batch_execute(r#"BEGIN; LOCK TABLE "LiteLLM_SpendLogs" IN ACCESS EXCLUSIVE MODE"#)
        .await
        .unwrap();
    holder
}

#[tokio::test]
async fn deletes_expired_rows_in_batches_until_exhausted() {
    let mut case = seeded(5, 2).await;
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let clock = StepClock::new(Duration::from_millis(1));
    let deadline = clock.at(1_000_000);
    let (result, batches) = run_table(
        &mut case,
        (BOUNDS, &clock, &metrics),
        CleanupTable::SpendLogs,
        (cutoff(), deadline),
    )
    .await;
    assert_eq!(result.stop_reason, StopReason::Exhausted);
    assert_eq!(result.rows_deleted, 5);
    assert_eq!(result.table, CleanupTable::SpendLogs);
    assert_eq!(batches, vec![2, 2, 1]);
    assert_eq!(row_count(&case.client, SPEND_LOGS).await, 2);
    assert_eq!(series_total(&registry, ROWS_DELETED), Some(finite(5)));
    assert_eq!(series_total(&registry, ROWS_REMAINING), Some(finite(0)));
    let durations = registry.get(BATCH_DURATION).unwrap();
    let samples = durations.series(&[Some(SPEND_LOGS.to_owned())]).unwrap();
    assert_eq!(samples.count, 4);
    let pooled = case
        .client
        .query_one("SHOW statement_timeout", &[])
        .await
        .unwrap();
    assert_eq!(pooled.get::<_, String>(0), "0");
    case.close().await;
}

#[tokio::test]
async fn stops_at_the_batch_cap_and_probes_a_capped_backlog() {
    let mut case = seeded(6, 1).await;
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let clock = StepClock::new(Duration::from_millis(1));
    let deadline = clock.at(1_000_000);
    let bounds = CleanupBounds {
        batch_size: 1,
        max_batches: 2,
        remaining_count_cap: 3,
        ..BOUNDS
    };
    let (result, batches) = run_table(
        &mut case,
        (bounds, &clock, &metrics),
        CleanupTable::SpendLogs,
        (cutoff(), deadline),
    )
    .await;
    assert_eq!(result.stop_reason, StopReason::BatchCapReached);
    assert_eq!(result.rows_deleted, 2);
    assert_eq!(batches, vec![1, 1]);
    assert_eq!(row_count(&case.client, SPEND_LOGS).await, 5);
    assert_eq!(series_total(&registry, ROWS_REMAINING), Some(finite(3)));
    case.close().await;
}

#[tokio::test]
async fn deletes_daily_rows_strictly_before_the_cutoff_day() {
    let mut case = SpendPartitionCase::provision().await;
    case.client.batch_execute(DAILY_TAGS_TABLE).await.unwrap();
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let clock = StepClock::new(Duration::from_millis(1));
    let deadline = clock.at(1_000_000);
    let day = Cutoff::Day(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    let (result, _batches) = run_table(
        &mut case,
        (BOUNDS, &clock, &metrics),
        CleanupTable::DailyTagSpend,
        (day, deadline),
    )
    .await;
    assert_eq!(result.stop_reason, StopReason::Exhausted);
    assert_eq!(result.rows_deleted, 1);
    let kept = case
        .client
        .query(
            r#"SELECT "id" FROM "LiteLLM_DailyTagSpend" ORDER BY "id""#,
            &[],
        )
        .await
        .unwrap();
    let ids: Vec<String> = kept.iter().map(|row| row.get::<_, String>(0)).collect();
    assert_eq!(ids, vec!["horizon".to_owned(), "new".to_owned()]);
    case.close().await;
}

#[tokio::test]
async fn checks_the_deadline_before_the_batch_cap() {
    let mut case = seeded(3, 0).await;
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.set_rows_remaining(SPEND_LOGS, 42);
    let clock = StepClock::new(Duration::from_millis(1));
    let deadline = clock.at(0);
    let bounds = CleanupBounds {
        max_batches: 0,
        ..BOUNDS
    };
    let (result, batches) = run_table(
        &mut case,
        (bounds, &clock, &metrics),
        CleanupTable::SpendLogs,
        (cutoff(), deadline),
    )
    .await;
    assert_eq!(result.stop_reason, StopReason::BudgetExhausted);
    assert_eq!(result.rows_deleted, 0);
    assert!(batches.is_empty());
    assert_eq!(row_count(&case.client, SPEND_LOGS).await, 3);
    assert_eq!(series_total(&registry, ROWS_REMAINING), Some(finite(42)));
    case.close().await;
}

#[tokio::test]
async fn aborts_after_consecutive_lock_timeouts_and_keeps_the_gauge() {
    let mut case = seeded(3, 0).await;
    let mut drivers = JoinSet::new();
    let holder = lock_spend_logs(case.fixture.options.url.expose_secret(), &mut drivers).await;
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.set_rows_remaining(SPEND_LOGS, 42);
    let clock = StepClock::new(Duration::from_millis(1));
    let deadline = clock.at(1_000_000);
    let bounds = CleanupBounds {
        batch_timeout: Duration::from_millis(50),
        max_batches: 1,
        ..BOUNDS
    };
    let (result, batches) = run_table(
        &mut case,
        (bounds, &clock, &metrics),
        CleanupTable::SpendLogs,
        (cutoff(), deadline),
    )
    .await;
    assert_eq!(result.stop_reason, StopReason::Aborted);
    assert_eq!(result.rows_deleted, 0);
    assert!(batches.is_empty());
    assert_eq!(series_total(&registry, BATCH_FAILURES), Some(finite(3)));
    assert_eq!(series_total(&registry, ROWS_REMAINING), Some(finite(42)));
    holder.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(row_count(&case.client, SPEND_LOGS).await, 3);
    drop(holder);
    drop(drivers);
    case.close().await;
}

#[tokio::test]
async fn a_successful_batch_resets_consecutive_failures() {
    let mut case = seeded(3, 0).await;
    case.client
        .batch_execute(ALTERNATING_FAILURES)
        .await
        .unwrap();
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let clock = StepClock::new(Duration::from_millis(1));
    let deadline = clock.at(1_000_000);
    let bounds = CleanupBounds {
        batch_size: 1,
        max_consecutive_failures: 2,
        ..BOUNDS
    };
    let (result, batches) = run_table(
        &mut case,
        (bounds, &clock, &metrics),
        CleanupTable::SpendLogs,
        (cutoff(), deadline),
    )
    .await;
    assert_eq!(result.stop_reason, StopReason::Exhausted);
    assert_eq!(batches, vec![1, 1, 1]);
    assert_eq!(series_total(&registry, BATCH_FAILURES), Some(finite(4)));
    case.close().await;
}

#[tokio::test]
async fn failed_batches_do_not_consume_the_batch_cap() {
    let mut case = seeded(3, 0).await;
    case.client
        .batch_execute(ALTERNATING_FAILURES)
        .await
        .unwrap();
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    let clock = StepClock::new(Duration::from_millis(1));
    let deadline = clock.at(1_000_000);
    let bounds = CleanupBounds {
        batch_size: 1,
        max_batches: 1,
        ..BOUNDS
    };
    let (result, batches) = run_table(
        &mut case,
        (bounds, &clock, &metrics),
        CleanupTable::SpendLogs,
        (cutoff(), deadline),
    )
    .await;
    assert_eq!(result.stop_reason, StopReason::BatchCapReached);
    assert_eq!(result.rows_deleted, 1);
    assert_eq!(batches, vec![1]);
    case.close().await;
}

#[tokio::test]
async fn a_batch_cancelled_at_the_deadline_is_not_a_failure() {
    let mut case = seeded(3, 0).await;
    let mut drivers = JoinSet::new();
    let holder = lock_spend_logs(case.fixture.options.url.expose_secret(), &mut drivers).await;
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.set_rows_remaining(SPEND_LOGS, 42);
    // readings: 0 checks the deadline, 1 starts the batch with 300 ms left, 2 sees the deadline passed
    let clock = StepClock::new(Duration::from_millis(600));
    let deadline = clock.start.checked_add(Duration::from_millis(900)).unwrap();
    let bounds = CleanupBounds {
        batch_timeout: Duration::from_secs(20),
        ..BOUNDS
    };
    let started = Instant::now();
    let (result, _batches) = run_table(
        &mut case,
        (bounds, &clock, &metrics),
        CleanupTable::SpendLogs,
        (cutoff(), deadline),
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(result.stop_reason, StopReason::BudgetExhausted);
    assert_eq!(series_total(&registry, BATCH_FAILURES), None);
    assert_eq!(series_total(&registry, ROWS_REMAINING), Some(finite(42)));
    holder.batch_execute("ROLLBACK").await.unwrap();
    drop(holder);
    drop(drivers);
    case.close().await;
}
