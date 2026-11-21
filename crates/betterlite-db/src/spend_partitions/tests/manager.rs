use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use secrecy::ExposeSecret as _;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_postgres::error::SqlState;
use tokio_postgres::{Client, NoTls};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::spend_partitions::SpendPartitionOperation;
use crate::spend_partitions::calendar::{PartitionClock, PartitionInterval, PlannedPartition};
use crate::spend_partitions::catalog::{
    BoundedTransactionError, TRANSACTION_COMMIT_SLACK, create_partition_statement,
    drop_partition_statement, execute_ddl, is_partitioned, list_partitions, within_deadline,
};
use crate::spend_partitions::fixtures::SpendPartitionCase;
use crate::spend_partitions::manager::{
    PartitionMaintenanceError, SpendPartitionManager, drop_partitions_older_than,
};
use crate::spend_partitions::partition_bound::{CatalogPartition, PartitionBound};

struct FixedClock(NaiveDate);

impl PartitionClock for FixedClock {
    fn today(&self) -> NaiveDate {
        self.0
    }
}

fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a calendar date")
}

fn midnight_utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    day(year, month, day)
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
}

fn stamp(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").expect("a wall-clock stamp")
}

const fn daily_manager(ahead: i64, today: NaiveDate) -> SpendPartitionManager<FixedClock> {
    SpendPartitionManager::new(PartitionInterval::Day, ahead, FixedClock(today))
}

fn budgets(values: Vec<Option<u64>>) -> impl FnMut() -> Option<u64> + Send {
    let mut remaining_values = values.into_iter();
    move || remaining_values.next().flatten()
}

fn strings(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|entry| (*entry).to_owned()).collect()
}

fn sorted(mut entries: Vec<String>) -> Vec<String> {
    entries.sort();
    entries
}

fn names(partitions: &[CatalogPartition]) -> Vec<String> {
    let collected = partitions
        .iter()
        .map(|partition| partition.name.clone())
        .collect();
    sorted(collected)
}

fn bound_of(partitions: &[CatalogPartition], name: &str) -> Option<PartitionBound> {
    partitions
        .iter()
        .find(|partition| partition.name == name)
        .map(|partition| partition.bound)
}

async fn partitioned_case() -> SpendPartitionCase {
    let case = SpendPartitionCase::provision().await;
    case.run_ok(SpendPartitionOperation::Partition).await;
    case
}

async fn partition_names(case: &mut SpendPartitionCase) -> Vec<String> {
    let listed = list_partitions(&mut case.client, 1_000)
        .await
        .expect("the partition list");
    names(&listed)
}

async fn ensured(
    case: &mut SpendPartitionCase,
    manager: &SpendPartitionManager<FixedClock>,
    values: Vec<Option<u64>>,
) -> Vec<String> {
    let mut budget = budgets(values);
    manager
        .ensure_partitions(&mut case.client, &mut budget)
        .await
}

async fn dropped(
    case: &mut SpendPartitionCase,
    cutoff: DateTime<Utc>,
    values: Vec<Option<u64>>,
) -> Result<Vec<String>, PartitionMaintenanceError> {
    let mut budget = budgets(values);
    drop_partitions_older_than(&mut case.client, cutoff, &mut budget).await
}

#[derive(Clone, Default)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("UTF-8 log lines")
    }
}

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogCapture {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn install_logs() -> (LogCapture, DefaultGuard) {
    let logs = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    let guard = set_default(subscriber);
    (logs, guard)
}

fn warned(logs: &str, message: &str) -> bool {
    logs.lines()
        .any(|line| line.contains("WARN") && line.contains(message))
}

async fn lock_parent(
    case: &SpendPartitionCase,
) -> (Client, JoinSet<Result<(), tokio_postgres::Error>>) {
    let url = case.fixture.options.url.expose_secret();
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("a lock connection");
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute(r#"BEGIN; LOCK TABLE "LiteLLM_SpendLogs" IN ACCESS EXCLUSIVE MODE"#)
        .await
        .expect("the parent lock");
    (client, tasks)
}

#[test]
fn create_statements_quote_catalog_identifiers() {
    let planned = PlannedPartition {
        name: r#"odd"name"#.to_owned(),
        lower: day(2026, 6, 2),
        upper: day(2026, 6, 3),
    };
    assert_eq!(
        create_partition_statement(&planned),
        r#"CREATE TABLE IF NOT EXISTS "odd""name" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2026-06-02') TO ('2026-06-03')"#
    );
    assert_eq!(
        drop_partition_statement(r#"odd"name"#),
        r#"DROP TABLE IF EXISTS "odd""name""#
    );
}

#[test]
fn the_commit_slack_is_five_seconds() {
    assert_eq!(TRANSACTION_COMMIT_SLACK, Duration::from_secs(5));
}

#[tokio::test]
async fn a_plain_table_is_never_converted() {
    let mut case = SpendPartitionCase::provision().await;
    let (logs, _guard) = install_logs();
    let mut budget = budgets(vec![Some(1_000)]);
    assert!(!is_partitioned(&mut case.client, &mut budget).await);
    assert!(partition_names(&mut case).await.is_empty());
    let ensured = ensured(
        &mut case,
        &daily_manager(1, day(2026, 6, 2)),
        vec![Some(1_000); 2],
    )
    .await;
    assert!(ensured.is_empty());
    let log = logs.contents();
    assert!(warned(
        &log,
        "Failed to ensure spend-log partition LiteLLM_SpendLogs_p20260602"
    ));
    assert!(warned(&log, "is not partitioned"));
    let relkind = case
        .client
        .query_one(
            r#"SELECT relkind::text AS relkind FROM pg_class WHERE oid = to_regclass('"LiteLLM_SpendLogs"')"#,
            &[],
        )
        .await
        .expect("the relation kind");
    assert_eq!(relkind.get::<_, String>("relkind"), "r");
    case.close().await;
}

#[tokio::test]
async fn the_probe_and_list_read_only_the_current_schema_and_need_a_budget() {
    let mut case = partitioned_case().await;
    let mut unspent = budgets(vec![None]);
    assert!(!is_partitioned(&mut case.client, &mut unspent).await);
    case.client
        .batch_execute(
            r#"CREATE SCHEMA partition_other;
CREATE TABLE partition_other."LiteLLM_SpendLogs" ("startTime" TIMESTAMP(3)) PARTITION BY RANGE ("startTime");
CREATE TABLE partition_other."LiteLLM_SpendLogs_pother" PARTITION OF partition_other."LiteLLM_SpendLogs" FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');"#,
        )
        .await
        .expect("the other schema");
    let mut budget = budgets(vec![Some(1_000)]);
    assert!(is_partitioned(&mut case.client, &mut budget).await);
    assert_eq!(
        partition_names(&mut case).await,
        strings(&["LiteLLM_SpendLogs_pdefault"])
    );
    case.client
        .batch_execute(r#"DROP TABLE "LiteLLM_SpendLogs""#)
        .await
        .expect("the current-schema parent drop");
    let mut after_drop_budget = budgets(vec![Some(1_000)]);
    assert!(!is_partitioned(&mut case.client, &mut after_drop_budget).await);
    assert!(partition_names(&mut case).await.is_empty());
    case.close().await;
}

#[tokio::test]
async fn probe_failures_assume_unpartitioned_and_list_failures_propagate() {
    let mut case = partitioned_case().await;
    let _terminated = case
        .client
        .batch_execute("SELECT pg_terminate_backend(pg_backend_pid())")
        .await;
    let (logs, _guard) = install_logs();
    let mut budget = budgets(vec![Some(1_000)]);
    assert!(!is_partitioned(&mut case.client, &mut budget).await);
    assert!(warned(
        &logs.contents(),
        "Could not determine if LiteLLM_SpendLogs is partitioned"
    ));
    let spent = dropped(&mut case, midnight_utc(2026, 6, 3), vec![None])
        .await
        .expect("a spent budget skips the list");
    assert!(spent.is_empty());
    let error = dropped(&mut case, midnight_utc(2026, 6, 3), vec![Some(1_000)])
        .await
        .expect_err("a dead connection");
    assert!(matches!(error, PartitionMaintenanceError::List { .. }));
    case.close().await;
}

#[tokio::test]
async fn ensure_creates_the_planned_partitions_and_names_them_each_run() {
    let mut case = partitioned_case().await;
    let manager = daily_manager(2, day(2026, 6, 1));
    let created = ensured(&mut case, &manager, vec![Some(1_000); 3]).await;
    assert_eq!(
        created,
        strings(&[
            "LiteLLM_SpendLogs_p20260601",
            "LiteLLM_SpendLogs_p20260602",
            "LiteLLM_SpendLogs_p20260603",
        ])
    );
    let listed = list_partitions(&mut case.client, 1_000)
        .await
        .expect("the partitions");
    assert_eq!(
        bound_of(&listed, "LiteLLM_SpendLogs_p20260601"),
        Some(PartitionBound::Naive(stamp("2026-06-02 00:00:00")))
    );
    assert_eq!(
        bound_of(&listed, "LiteLLM_SpendLogs_p20260603"),
        Some(PartitionBound::Naive(stamp("2026-06-04 00:00:00")))
    );
    let again = ensured(&mut case, &manager, vec![Some(1_000); 3]).await;
    assert_eq!(again, created);
    assert_eq!(
        partition_names(&mut case).await,
        strings(&[
            "LiteLLM_SpendLogs_p20260601",
            "LiteLLM_SpendLogs_p20260602",
            "LiteLLM_SpendLogs_p20260603",
            "LiteLLM_SpendLogs_pdefault",
        ])
    );
    case.close().await;
}

#[tokio::test]
async fn ensure_stops_when_the_budget_is_spent() {
    let mut case = partitioned_case().await;
    let manager = daily_manager(2, day(2026, 6, 1));
    let created = ensured(&mut case, &manager, vec![Some(1_000), None]).await;
    assert_eq!(created, strings(&["LiteLLM_SpendLogs_p20260601"]));
    assert_eq!(
        partition_names(&mut case).await,
        strings(&["LiteLLM_SpendLogs_p20260601", "LiteLLM_SpendLogs_pdefault"])
    );
    let resumed = ensured(&mut case, &manager, vec![Some(1_000); 3]).await;
    assert_eq!(resumed.len(), 3);
    let later = daily_manager(0, day(2026, 6, 10));
    let skipped = ensured(&mut case, &later, vec![None]).await;
    assert!(skipped.is_empty());
    assert_eq!(partition_names(&mut case).await.len(), 4);
    case.close().await;
}

#[tokio::test]
async fn ensure_continues_after_one_partition_overlaps() {
    let mut case = partitioned_case().await;
    case.client
        .batch_execute(
            r#"CREATE TABLE "LiteLLM_SpendLogs_manual" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2026-06-02') TO ('2026-06-03')"#,
        )
        .await
        .expect("an overlapping partition");
    let (logs, _guard) = install_logs();
    let created = ensured(
        &mut case,
        &daily_manager(1, day(2026, 6, 2)),
        vec![Some(1_000); 2],
    )
    .await;
    assert_eq!(created, strings(&["LiteLLM_SpendLogs_p20260603"]));
    let log = logs.contents();
    assert!(warned(
        &log,
        "Failed to ensure spend-log partition LiteLLM_SpendLogs_p20260602"
    ));
    assert!(warned(&log, "would overlap"));
    assert_eq!(
        partition_names(&mut case).await,
        strings(&[
            "LiteLLM_SpendLogs_manual",
            "LiteLLM_SpendLogs_p20260603",
            "LiteLLM_SpendLogs_pdefault",
        ])
    );
    case.close().await;
}

#[tokio::test]
async fn a_locked_parent_cancels_the_create_within_the_statement_budget() {
    let mut case = partitioned_case().await;
    let (lock_client, _lock_tasks) = lock_parent(&case).await;
    let planned = PlannedPartition {
        name: "LiteLLM_SpendLogs_p20260602".to_owned(),
        lower: day(2026, 6, 2),
        upper: day(2026, 6, 3),
    };
    let started = Instant::now();
    let statement = create_partition_statement(&planned);
    let ddl = execute_ddl(&mut case.client, &statement, 200);
    let outcome = timeout(Duration::from_secs(5), within_deadline(200, ddl))
        .await
        .expect("the create answers within the client deadline");
    let elapsed = started.elapsed();
    let error = outcome.expect_err("a create waiting on the parent lock");
    assert!(matches!(
        error,
        BoundedTransactionError::Database(source)
            if source.code() == Some(&SqlState::QUERY_CANCELED)
    ));
    assert!(elapsed >= Duration::from_millis(150), "{elapsed:?}");
    assert!(elapsed < TRANSACTION_COMMIT_SLACK, "{elapsed:?}");
    lock_client
        .batch_execute("COMMIT")
        .await
        .expect("the lock release");
    case.close().await;
}

#[tokio::test]
async fn drops_partitions_at_or_before_the_cutoff_and_keeps_default() {
    let mut case = partitioned_case().await;
    let manager = daily_manager(2, day(2026, 6, 1));
    assert_eq!(
        ensured(&mut case, &manager, vec![Some(1_000); 3])
            .await
            .len(),
        3
    );
    let removed = dropped(&mut case, midnight_utc(2026, 6, 3), vec![Some(1_000); 3])
        .await
        .expect("the expired partitions");
    assert_eq!(
        sorted(removed),
        strings(&["LiteLLM_SpendLogs_p20260601", "LiteLLM_SpendLogs_p20260602",])
    );
    assert_eq!(
        partition_names(&mut case).await,
        strings(&["LiteLLM_SpendLogs_p20260603", "LiteLLM_SpendLogs_pdefault"])
    );
    case.close().await;
}

#[tokio::test]
async fn drop_stops_when_the_budget_is_spent() {
    let mut case = partitioned_case().await;
    let manager = daily_manager(2, day(2026, 6, 1));
    assert_eq!(
        ensured(&mut case, &manager, vec![Some(1_000); 3])
            .await
            .len(),
        3
    );
    let removed = dropped(
        &mut case,
        midnight_utc(2026, 6, 3),
        vec![Some(1_000), Some(1_000), None],
    )
    .await
    .expect("the first expired partition");
    assert_eq!(removed.len(), 1);
    assert_eq!(partition_names(&mut case).await.len(), 3);
    case.close().await;
}

#[tokio::test]
async fn drop_quotes_a_partition_name_that_contains_a_quote() {
    let mut case = partitioned_case().await;
    case.client
        .batch_execute(
            r#"CREATE TABLE "LiteLLM_SpendLogs_o""dd" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2026-06-01') TO ('2026-06-02')"#,
        )
        .await
        .expect("a quoted partition name");
    let removed = dropped(&mut case, midnight_utc(2026, 6, 3), vec![Some(1_000); 2])
        .await
        .expect("the quoted partition");
    assert_eq!(removed, strings(&[r#"LiteLLM_SpendLogs_o"dd"#]));
    assert_eq!(
        partition_names(&mut case).await,
        strings(&["LiteLLM_SpendLogs_pdefault"])
    );
    case.close().await;
}

#[tokio::test]
async fn drop_continues_after_one_partition_cannot_be_dropped() {
    let mut case = partitioned_case().await;
    let manager = daily_manager(1, day(2026, 6, 1));
    assert_eq!(
        ensured(&mut case, &manager, vec![Some(1_000); 2])
            .await
            .len(),
        2
    );
    case.client
        .batch_execute(
            r#"CREATE VIEW "spend_logs_view" AS SELECT * FROM "LiteLLM_SpendLogs_p20260601""#,
        )
        .await
        .expect("a dependent view");
    let (logs, _guard) = install_logs();
    let removed = dropped(&mut case, midnight_utc(2026, 6, 3), vec![Some(1_000); 3])
        .await
        .expect("the remaining drops");
    assert_eq!(removed, strings(&["LiteLLM_SpendLogs_p20260602"]));
    assert!(warned(
        &logs.contents(),
        "Failed to drop spend-log partition LiteLLM_SpendLogs_p20260601"
    ));
    assert_eq!(
        partition_names(&mut case).await,
        strings(&["LiteLLM_SpendLogs_p20260601", "LiteLLM_SpendLogs_pdefault"])
    );
    case.close().await;
}

#[tokio::test]
async fn drop_rejects_an_aware_upper_bound_before_removing_anything() {
    let mut case = SpendPartitionCase::provision().await;
    case.client
        .batch_execute(
            r#"DROP TABLE "LiteLLM_SpendLogs";
CREATE TABLE "LiteLLM_SpendLogs" ("startTime" TIMESTAMP WITH TIME ZONE) PARTITION BY RANGE ("startTime");
CREATE TABLE "LiteLLM_SpendLogs_p20260601" PARTITION OF "LiteLLM_SpendLogs" FOR VALUES FROM ('2026-06-01T00:00:00+00:00') TO ('2026-06-02T00:00:00+00:00');"#,
        )
        .await
        .expect("a timestamptz partition");
    let error = dropped(&mut case, midnight_utc(2026, 6, 3), vec![Some(1_000)])
        .await
        .expect_err("the offset bound");
    assert!(matches!(error, PartitionMaintenanceError::Bound(_)));
    assert!(error.to_string().contains("carries a UTC offset"));
    let present: bool = case
        .client
        .query_one(
            r#"SELECT to_regclass('"LiteLLM_SpendLogs_p20260601"') IS NOT NULL AS present"#,
            &[],
        )
        .await
        .expect("the partition presence")
        .get("present");
    assert!(present);
    case.close().await;
}
