#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use num_traits::ToPrimitive as _;
use secrecy::ExposeSecret as _;
use tokio::task::{JoinSet, yield_now};
use tokio::time::Instant;
use tokio_postgres::{Client, NoTls};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use crate::cleanup_metrics::CleanupMetrics;
use crate::queues::pod_lock::owner::PodId;
use crate::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use crate::spend_partitions::catalog::{BoundedTransactionError, RemainingBudget};
use crate::spend_partitions::fixtures::SpendPartitionCase;
use crate::spend_partitions::manager::PartitionMaintenanceError;
use crate::spend_retention::batches::deadline::MonotonicClock;
use crate::spend_retention::batches::settings::{
    CleanupBounds, CleanupGeneralSettings, CleanupSettings, DurationParser,
};
use crate::spend_retention::runner::clock::CleanupClock;
use crate::spend_retention::runner::error::LockReleaseError;
use crate::spend_retention::runner::groups::GroupRun;
use crate::spend_retention::runner::lock::CleanupLock;
use crate::spend_retention::runner::partitions::PartitionMaintenance;
use crate::spend_retention::runner::progress::RunProgress;

pub(crate) const RUNS: &str = "litellm_spend_log_cleanup_runs_total";

pub(crate) const BOUNDS: CleanupBounds = CleanupBounds {
    batch_size: 2,
    max_batches: 500,
    run_budget: Duration::from_secs(300),
    batch_timeout: Duration::from_secs(600),
    max_consecutive_failures: 3,
    failure_backoff: Duration::ZERO,
    remaining_count_cap: 100_000,
};

const OTHER_TABLES: &str = r#"CREATE TABLE "LiteLLM_SpendLogToolIndex" ("request_id" TEXT NOT NULL, "tool_name" TEXT NOT NULL, "start_time" TIMESTAMPTZ NOT NULL);
CREATE TABLE "LiteLLM_AutoRouterSession" ("api_key" TEXT NOT NULL, "session_id" TEXT NOT NULL, "router_name" TEXT NOT NULL, "last_turn_at" TIMESTAMPTZ NOT NULL);
CREATE TABLE "LiteLLM_AutoRouterUserSession" ("user_id" TEXT NOT NULL, "api_key" TEXT NOT NULL, "session_id" TEXT NOT NULL, "router_name" TEXT NOT NULL, "last_turn_at" TIMESTAMPTZ NOT NULL);
CREATE TABLE "LiteLLM_HealthCheckTable" ("health_check_id" TEXT NOT NULL, "checked_at" TIMESTAMPTZ NOT NULL);
CREATE TABLE "LiteLLM_DailyTagSpend" ("id" TEXT NOT NULL, "date" TEXT NOT NULL);"#;

const MANY_SPEND_LOGS: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime")
SELECT 'row-' || g, 'completion', 'sk-fixture', $1, $1 FROM generate_series(1, $2) AS g"#;

const TOOL_INDEX_ROW: &str = r#"INSERT INTO "LiteLLM_SpendLogToolIndex" (request_id, "tool_name", "start_time") VALUES ($1, 'tool', $2)"#;
const MANY_TOOL_INDEX_ROWS: &str = r#"INSERT INTO "LiteLLM_SpendLogToolIndex" (request_id, "tool_name", "start_time")
SELECT 'tool-' || g, 'tool', $1::timestamptz FROM generate_series(1, $2) AS g"#;
const MANY_SESSION_ROWS: &str = r#"INSERT INTO "LiteLLM_AutoRouterSession" (api_key, session_id, router_name, last_turn_at)
SELECT 'sk-key', 'session-' || g, 'router', $1::timestamptz FROM generate_series(1, $2) AS g"#;
const MANY_USER_SESSION_ROWS: &str = r#"INSERT INTO "LiteLLM_AutoRouterUserSession" (user_id, api_key, session_id, router_name, last_turn_at)
SELECT 'user', 'sk-key', 'user-session-' || g, 'router', $1::timestamptz FROM generate_series(1, $2) AS g"#;
const SESSION_ROW: &str =
    r#"INSERT INTO "LiteLLM_AutoRouterSession" VALUES ($1, $2, 'router', $3)"#;
const USER_SESSION_ROW: &str =
    r#"INSERT INTO "LiteLLM_AutoRouterUserSession" VALUES ('user', $1, $2, 'router', $3)"#;
const HEALTH_CHECK_ROW: &str = r#"INSERT INTO "LiteLLM_HealthCheckTable" VALUES ($1, $2)"#;
const DAILY_TAG_ROW: &str = r#"INSERT INTO "LiteLLM_DailyTagSpend" VALUES ($1, $2)"#;

pub(crate) const SPEND_IDS: &str =
    r#"SELECT request_id FROM "LiteLLM_SpendLogs" ORDER BY request_id"#;
pub(crate) const TOOL_IDS: &str =
    r#"SELECT request_id FROM "LiteLLM_SpendLogToolIndex" ORDER BY request_id"#;
pub(crate) const SESSION_IDS: &str =
    r#"SELECT session_id FROM "LiteLLM_AutoRouterSession" ORDER BY session_id"#;
pub(crate) const USER_SESSION_IDS: &str =
    r#"SELECT session_id FROM "LiteLLM_AutoRouterUserSession" ORDER BY session_id"#;
pub(crate) const HEALTH_IDS: &str =
    r#"SELECT health_check_id FROM "LiteLLM_HealthCheckTable" ORDER BY health_check_id"#;
pub(crate) const TAG_IDS: &str = r#"SELECT "id" FROM "LiteLLM_DailyTagSpend" ORDER BY "id""#;
pub(crate) const PARTITION_NAMES: &str = r#"SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = current_schema() AND c.relname LIKE 'LiteLLM\_SpendLogs\_%' ORDER BY c.relname"#;

// the SpendLogs table comes from the partition fixture; the runner's other five tables are created beside it
pub(crate) async fn provision() -> SpendPartitionCase {
    let case = SpendPartitionCase::provision().await;
    case.client
        .batch_execute(OTHER_TABLES)
        .await
        .expect("the cleanup tables");
    case
}

// a second connection, so a test can watch a run's committed batches while the run holds the first
pub(crate) async fn observer(
    case: &SpendPartitionCase,
) -> (Client, JoinSet<Result<(), tokio_postgres::Error>>) {
    let url = case.fixture.options.url.expose_secret();
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("an observer connection");
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    (client, tasks)
}

// the remaining rows of one text column, in the order the query asked for, so an assert reads as the list
pub(crate) async fn text_join(client: &Client, sql: &str) -> String {
    let rows = client.query(sql, &[]).await.expect("the remaining rows");
    rows.iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<String>>()
        .join(",")
}

// the collaborators one group reads, in the shape the group entry points take
pub(super) fn group_run<'run, Clock>(
    client: &'run mut Client,
    clock: &'run Clock,
    metrics: &'run CleanupMetrics<'run>,
    progress: &'run RunProgress,
    started: Instant,
) -> GroupRun<'run, 'run, Clock> {
    GroupRun {
        client,
        clock,
        metrics,
        bounds: BOUNDS,
        progress,
        started,
    }
}

pub(crate) const SPEND_COUNT: &str = r#"SELECT count(*) FROM "LiteLLM_SpendLogs""#;
pub(crate) const SESSION_COUNT: &str = r#"SELECT count(*) FROM "LiteLLM_AutoRouterSession""#;

pub(crate) async fn count_rows(client: &Client, sql: &str) -> i64 {
    let row = client.query_one(sql, &[]).await.expect("the row count");
    row.get::<_, i64>(0)
}

pub(crate) async fn seed_spend_logs(case: &SpendPartitionCase, at: NaiveDateTime, count: i64) {
    case.client
        .execute(MANY_SPEND_LOGS, &[&at, &count])
        .await
        .expect("seeded spend logs");
}

pub(crate) async fn seed_tool_index(case: &SpendPartitionCase, id: &str, at: DateTime<Utc>) {
    case.client
        .execute(TOOL_INDEX_ROW, &[&id, &at])
        .await
        .expect("a tool index row");
}

pub(crate) async fn seed_tool_indexes(case: &SpendPartitionCase, at: DateTime<Utc>, count: i64) {
    case.client
        .execute(MANY_TOOL_INDEX_ROWS, &[&at, &count])
        .await
        .expect("seeded tool index rows");
}

pub(crate) async fn seed_sessions(case: &SpendPartitionCase, at: DateTime<Utc>, count: i64) {
    case.client
        .execute(MANY_SESSION_ROWS, &[&at, &count])
        .await
        .expect("seeded session rows");
}

pub(crate) async fn seed_user_sessions(case: &SpendPartitionCase, at: DateTime<Utc>, count: i64) {
    case.client
        .execute(MANY_USER_SESSION_ROWS, &[&at, &count])
        .await
        .expect("seeded user session rows");
}

pub(crate) async fn seed_session(case: &SpendPartitionCase, id: &str, at: DateTime<Utc>) {
    case.client
        .execute(SESSION_ROW, &[&"sk-key", &id, &at])
        .await
        .expect("a session row");
}

pub(crate) async fn seed_user_session(case: &SpendPartitionCase, id: &str, at: DateTime<Utc>) {
    case.client
        .execute(USER_SESSION_ROW, &[&"sk-key", &id, &at])
        .await
        .expect("a user session row");
}

pub(crate) async fn seed_health_check(case: &SpendPartitionCase, id: &str, at: DateTime<Utc>) {
    case.client
        .execute(HEALTH_CHECK_ROW, &[&id, &at])
        .await
        .expect("a health-check row");
}

pub(crate) async fn seed_daily_tag(case: &SpendPartitionCase, id: &str, day: &str) {
    case.client
        .execute(DAILY_TAG_ROW, &[&id, &day])
        .await
        .expect("a daily tag row");
}

pub(crate) fn naive(year: i32, month: u32, day: u32, hour: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .expect("a calendar date")
        .and_hms_opt(hour, 0, 0)
        .expect("a wall-clock time")
}

pub(crate) fn moment(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(year, month, day)
        .expect("a calendar date")
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
}

pub(crate) fn retention_settings(json: &str) -> CleanupSettings {
    let general = serde_json::from_str::<CleanupGeneralSettings>(json)
        .expect("the general_settings overrides");
    CleanupSettings {
        general,
        defaults: BOUNDS,
    }
}

pub(crate) struct FixedCleanupClock {
    pub(crate) utc: DateTime<Utc>,
    pub(crate) mono: Instant,
}

impl MonotonicClock for FixedCleanupClock {
    fn now(&self) -> Instant {
        self.mono
    }
}

impl CleanupClock for FixedCleanupClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.utc
    }
}

// each reading moves the clock one step, so a test places a deadline between two known readings
pub(crate) struct StepCleanupClock {
    pub(crate) utc: DateTime<Utc>,
    pub(crate) start: Instant,
    pub(crate) step: Duration,
    pub(crate) readings: AtomicU32,
}

impl StepCleanupClock {
    pub(crate) fn reading(&self, steps: u32) -> Instant {
        let Some(offset) = self.step.checked_mul(steps) else {
            panic!("the test clock step overflows");
        };
        self.start.checked_add(offset).expect("a clock reading")
    }
}

impl MonotonicClock for StepCleanupClock {
    fn now(&self) -> Instant {
        let reading = self.readings.fetch_add(1, Ordering::SeqCst);
        self.reading(reading)
    }
}

impl CleanupClock for StepCleanupClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.utc
    }
}

impl MonotonicClock for Arc<StepCleanupClock> {
    fn now(&self) -> Instant {
        self.as_ref().now()
    }
}

impl CleanupClock for Arc<StepCleanupClock> {
    fn utc_now(&self) -> DateTime<Utc> {
        self.utc
    }
}

pub(crate) struct TestDurations;

impl DurationParser for TestDurations {
    type Failure = String;

    fn duration_seconds(&self, spec: &str) -> Result<i64, String> {
        match spec {
            "1d" => Ok(86_400),
            "3d" => Ok(259_200),
            "8d" => Ok(691_200),
            "10d" => Ok(864_000),
            "30d" => Ok(2_592_000),
            "huge" => Ok(i64::MAX),
            other => Err(format!("unknown duration {other}")),
        }
    }
}

#[derive(Default)]
pub(crate) struct FakeLock {
    pub(crate) backed: bool,
    pub(crate) outcome: Option<bool>,
    pub(crate) release_failure: bool,
    pub(crate) calls: Mutex<Vec<&'static str>>,
}

impl FakeLock {
    // B1690's manager with a Redis cache whose lease this run takes
    pub(crate) fn backed(outcome: bool) -> Self {
        Self {
            backed: true,
            outcome: Some(outcome),
            release_failure: false,
            calls: Mutex::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CleanupLock for FakeLock {
    fn redis_backed(&self) -> bool {
        self.backed
    }

    async fn acquire(&self, _cronjob_id: &str) -> Option<bool> {
        yield_now().await;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("acquire");
        self.outcome
    }

    async fn release(&self, _cronjob_id: &str) -> Result<(), LockReleaseError> {
        yield_now().await;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("release");
        if self.release_failure {
            return Err(LockReleaseError);
        }
        Ok(())
    }
}

// a lease holder that spends wall-clock readings before it grants, so a test can tell when the budget started
pub(crate) struct SlowLock {
    pub(crate) clock: Arc<StepCleanupClock>,
    pub(crate) readings: u32,
}

impl CleanupLock for SlowLock {
    fn redis_backed(&self) -> bool {
        true
    }

    async fn acquire(&self, _cronjob_id: &str) -> Option<bool> {
        yield_now().await;
        for _step in 0..self.readings {
            let _reading = self.clock.now();
        }
        Some(true)
    }

    async fn release(&self, _cronjob_id: &str) -> Result<(), LockReleaseError> {
        yield_now().await;
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct FakePartitions {
    pub(crate) partitioned: bool,
    pub(crate) failure: bool,
    pub(crate) calls: Mutex<Vec<&'static str>>,
    pub(crate) seen_remaining: Mutex<Vec<u64>>,
}

impl FakePartitions {
    // a table that is already partitioned, with a healthy or a failing partition list
    pub(crate) fn enabled(failure: bool) -> Self {
        Self {
            partitioned: true,
            failure,
            calls: Mutex::default(),
            seen_remaining: Mutex::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn seen_remaining(&self) -> Vec<u64> {
        self.seen_remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PartitionMaintenance for FakePartitions {
    async fn is_partitioned(&self, _client: &mut Client, remaining: RemainingBudget<'_>) -> bool {
        yield_now().await;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("probe");
        let Some(timeout_ms) = remaining() else {
            return self.partitioned;
        };
        self.seen_remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(timeout_ms);
        self.partitioned
    }

    async fn ensure_upcoming(&self, _client: &mut Client, _remaining: RemainingBudget<'_>) {
        yield_now().await;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("ensure");
    }

    async fn drop_expired(
        &self,
        _client: &mut Client,
        _cutoff: DateTime<Utc>,
        _remaining: RemainingBudget<'_>,
    ) -> Result<Vec<String>, PartitionMaintenanceError> {
        yield_now().await;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("drop");
        if self.failure {
            return Err(PartitionMaintenanceError::List {
                source: BoundedTransactionError::Deadline { millis: 1 },
            });
        }
        Ok(vec!["dropped".to_owned()])
    }
}

pub(crate) fn runs(registry: &CollectorRegistry, outcome: &str) -> Option<i64> {
    let series = registry
        .get(RUNS)
        .and_then(|metric| metric.series(&[Some(outcome.to_owned())]))?;
    match series.total {
        MetricValue::Finite(count) => count.to_i64(),
        MetricValue::PositiveInfinity | MetricValue::NegativeInfinity | MetricValue::NotANumber => {
            None
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub(crate) fn contents(&self) -> String {
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

pub(crate) fn install_logs() -> (LogCapture, DefaultGuard) {
    let logs = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    let guard = set_default(subscriber);
    (logs, guard)
}

// a lease cache that stores the owner it was given and records every delete, so a release leaves a trace
#[derive(Default)]
pub(crate) struct LeaseCache {
    owner: Mutex<Option<String>>,
    deleted: Mutex<Vec<String>>,
}

impl LeaseCache {
    pub(crate) fn deleted(&self) -> Vec<String> {
        self.deleted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl LockCache for LeaseCache {
    async fn get_owner(&self, _key: &str) -> Result<Option<Vec<u8>>, LockCacheError> {
        yield_now().await;
        let owner = self
            .owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(owner.map(String::into_bytes))
    }

    async fn set_owner_if_absent(
        &self,
        _key: &str,
        owner: &str,
        _ttl_seconds: i64,
    ) -> Result<bool, LockCacheError> {
        yield_now().await;
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = Some(owner.to_owned());
        Ok(true)
    }

    async fn delete(&self, key: &str) -> Result<i64, LockCacheError> {
        yield_now().await;
        self.deleted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        Ok(1)
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

// B1690's manager over a cache that always grants, which is the lease the runner's lock seam wraps
pub(crate) fn lease_manager(cache: Option<Arc<LeaseCache>>) -> PodLockManager<LeaseCache> {
    PodLockManager::new(
        cache,
        60,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(Uuid::new_v4()).expect("a v4 pod id"),
    )
}
