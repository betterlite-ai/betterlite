#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Mutex as WriteLock;
use tokio::task::{JoinSet, yield_now};
use tokio::time::{sleep, timeout};
use tokio_postgres::NoTls;

use super::{MAX_DRAIN_PASSES, MonitorStartError, SpendLogsMonitorSlot, drain_spend_logs_queue};
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::gateway_request_tracking::test_support::captured_logs;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::events::NoRedisBufferEventSink;
use crate::redis_spend::keys::REDIS_SPEND_LOGS_BUFFER_KEY;
use crate::redis_spend::lists::SpendBufferLists;
use crate::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use crate::spend_commit::daily::postgres::PostgresDailyChunks;
use crate::spend_commit::policy::SpendFailure;
use crate::spend_flush::daily::{DailyCommitParts, DailyFlushError, flush_daily_spend_queue};
use crate::spend_flush::fakes::tests::{
    ChunkStep, Events, FakeDailyWriter, FakeService, FakeServiceError, FixedClock, FixedJitter,
    PauseLog, RecordingLists,
};
use crate::spend_flush::monitor::SpendMonitorSettings;
use crate::spend_log_storage::rows::writer::NoSpendLogFailureSink;
use crate::spend_log_storage::rows::{QueuePosition, SpendLogQueue};
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};
use crate::spend_queues::events::NoSpendQueueEventSink;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

struct DailyCase {
    events: Arc<Events>,
    queue: Arc<DailySpendUpdateQueue>,
    writer: Arc<FakeDailyWriter>,
    parts: Arc<DailyCommitParts<FakeDailyWriter>>,
    commits: TaskTracker,
    shutdown: CancellationToken,
}

// The commit ports one case needs: the writer under test, a fixed clock and jitter, no retries and a retry pause that records nothing.
fn commit_parts<Writer>(writer: Arc<Writer>) -> Arc<DailyCommitParts<Writer>> {
    Arc::new(DailyCommitParts {
        writer,
        retries: 0,
        clock: Arc::new(FixedClock {
            instant: Instant::now(),
        }),
        jitter: Arc::new(FixedJitter),
        pause: Arc::new(PauseLog::default()),
        failures: Arc::new(NoSpendLogFailureSink),
    })
}

impl DailyCase {
    fn new(steps: Vec<ChunkStep>) -> Self {
        let events = Arc::new(Events::default());
        let writer = Arc::new(FakeDailyWriter::new(Arc::clone(&events), steps));
        let parts = commit_parts(Arc::clone(&writer));
        Self {
            events,
            writer,
            parts,
            commits: TaskTracker::new(),
            shutdown: CancellationToken::new(),
            queue: Arc::new(
                DailySpendUpdateQueue::new(100, 1000, 100, Arc::new(NoSpendQueueEventSink))
                    .unwrap(),
            ),
        }
    }

    async fn enqueue(&self, keys: &[String]) {
        let rows: DailySpendUpdate = keys
            .iter()
            .map(|key| (key.clone(), SpendRow::new()))
            .collect();
        self.queue.add_update(rows).await.unwrap();
    }

    // The flush runs as the shutdown's own task, so the test can cancel while it is still committing.
    fn spawn(&self, tasks: &mut JoinSet<Result<(), DailyFlushError>>) {
        let queue = Arc::clone(&self.queue);
        let parts = Arc::clone(&self.parts);
        let commits = self.commits.clone();
        let shutdown = self.shutdown.clone();
        tasks.spawn(async move {
            flush_daily_spend_queue(&queue, DailySpendEntity::User, &parts, &commits, &shutdown)
                .await
        });
    }
}

fn key_range(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("key-{index:03}")).collect()
}

#[tokio::test]
async fn a_cancel_before_the_first_chunk_returns_the_whole_map() {
    let case = DailyCase::new(vec![ChunkStep::WaitThenCommit]);
    case.enqueue(&key_range(2)).await;
    let mut tasks = JoinSet::new();
    case.spawn(&mut tasks);
    timeout(Duration::from_secs(5), case.writer.entered().notified())
        .await
        .unwrap();

    case.shutdown.cancel();
    yield_now().await;
    case.writer.release().notify_one();
    let settled = tasks.join_next().await.unwrap().unwrap();

    assert!(matches!(settled, Err(DailyFlushError::Cancelled { .. })));
    let requeued = case.queue.flush_aggregated_daily_rows().await.unwrap();
    assert_eq!(requeued.keys().collect::<Vec<_>>(), ["key-000", "key-001"]);
    assert_eq!(case.events.entries(), ["daily User 2 WaitThenCommit"]);
}

#[tokio::test]
async fn a_cancel_after_the_first_chunk_returns_only_the_rows_left() {
    let case = DailyCase::new(vec![ChunkStep::Commit, ChunkStep::WaitThenCommit]);
    case.enqueue(&key_range(101)).await;
    let mut tasks = JoinSet::new();
    case.spawn(&mut tasks);
    timeout(Duration::from_secs(5), case.writer.entered().notified())
        .await
        .unwrap();

    case.shutdown.cancel();
    yield_now().await;
    case.writer.release().notify_one();
    let settled = tasks.join_next().await.unwrap().unwrap();

    assert!(matches!(settled, Err(DailyFlushError::Cancelled { .. })));
    let requeued = case.queue.flush_aggregated_daily_rows().await.unwrap();
    assert_eq!(requeued.keys().collect::<Vec<_>>(), ["key-100"]);
}

#[tokio::test]
async fn a_commit_interrupted_mid_statement_is_left_to_the_queue() {
    let case = DailyCase::new(vec![ChunkStep::BeginThenWaitThenCommit]);
    case.enqueue(&key_range(1)).await;
    let mut tasks = JoinSet::new();
    case.spawn(&mut tasks);
    timeout(Duration::from_secs(5), case.writer.entered().notified())
        .await
        .unwrap();

    case.shutdown.cancel();
    yield_now().await;
    let early = tasks.try_join_next().unwrap().unwrap();

    assert!(matches!(early, Err(DailyFlushError::Cancelled { .. })));
    case.writer.release().notify_one();
    case.queue.settle_interrupted_commits().await;
    assert!(
        case.queue
            .flush_aggregated_daily_rows()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_interrupted_commit_that_fails_returns_its_rows_when_it_settles() {
    let case = DailyCase::new(vec![ChunkStep::BeginThenWaitThenFail(SpendFailure::Other)]);
    case.enqueue(&key_range(2)).await;
    let mut tasks = JoinSet::new();
    case.spawn(&mut tasks);
    timeout(Duration::from_secs(5), case.writer.entered().notified())
        .await
        .unwrap();

    case.shutdown.cancel();
    yield_now().await;
    assert!(tasks.try_join_next().is_some());
    case.writer.release().notify_one();
    case.queue.settle_interrupted_commits().await;

    let requeued = case.queue.flush_aggregated_daily_rows().await.unwrap();
    assert_eq!(requeued.keys().collect::<Vec<_>>(), ["key-000", "key-001"]);
}

// The shutdown drain over a fake service, one queue and one Redis port.
struct DrainCase {
    events: Arc<Events>,
    service: Arc<FakeService>,
    queue: Arc<SpendLogQueue>,
    lists: Arc<RecordingLists>,
    lock: WriteLock<()>,
    slot: SpendLogsMonitorSlot,
}

impl DrainCase {
    fn new(census: Vec<i64>) -> Self {
        let events = Arc::new(Events::default());
        let service = Arc::new(FakeService::new(Arc::clone(&events), census));
        let queue = Arc::new(SpendLogQueue::new(usize::MAX));
        let lists = Arc::new(RecordingLists::default());
        Self {
            events,
            service,
            queue,
            lists,
            lock: WriteLock::new(()),
            slot: SpendLogsMonitorSlot::new(),
        }
    }

    async fn drain(&self, redis: &RedisUpdateBuffer) -> Result<(), FakeServiceError> {
        drain_spend_logs_queue(
            &self.slot,
            &self.lock,
            self.service.as_ref(),
            &self.queue,
            redis,
        )
        .await
    }

    fn enqueue(&self, request_ids: &[&str]) {
        let rows: Vec<SpendLogRow> = request_ids
            .iter()
            .map(|request_id| {
                SpendLogRow::from([(
                    "request_id".to_owned(),
                    SpendLogValue::Text((*request_id).to_owned()),
                )])
            })
            .collect();
        self.queue.enqueue(&rows, QueuePosition::Tail);
    }
}

fn redis_buffer(lists: &Arc<RecordingLists>) -> RedisUpdateBuffer {
    let port: Arc<dyn SpendBufferLists> = Arc::clone(lists);
    RedisUpdateBuffer {
        lists: Some(port),
        enabled: true,
        dequeue_count: 5,
        events: Arc::new(NoRedisBufferEventSink),
    }
}

fn memory_only_buffer() -> RedisUpdateBuffer {
    RedisUpdateBuffer {
        lists: None,
        enabled: false,
        dequeue_count: 5,
        events: Arc::new(NoRedisBufferEventSink),
    }
}

fn monitor_settings(base_interval: Duration) -> SpendMonitorSettings {
    SpendMonitorSettings {
        queue_size_threshold: 100,
        base_interval,
    }
}

#[tokio::test]
async fn a_shutdown_joins_the_monitor_before_its_first_drain_pass() {
    let case = DrainCase::new(vec![100, 0]);
    case.service.block_updates();
    case.slot
        .start(
            Arc::clone(&case.service),
            Arc::clone(&case.queue),
            monitor_settings(Duration::from_secs(2)),
        )
        .unwrap();
    timeout(Duration::from_secs(5), case.service.entered().notified())
        .await
        .unwrap();
    assert!(case.slot.is_running());

    let outcome = case.drain(&redis_buffer(&case.lists)).await;

    outcome.unwrap();
    assert!(!case.slot.is_running());
    assert_eq!(
        case.events.entries(),
        ["recover", "census 100", "update Block", "census 0"]
    );
    case.service.release().notify_one();
    yield_now().await;
    yield_now().await;
    assert_eq!(
        case.events.entries(),
        ["recover", "census 100", "update Block", "census 0"]
    );
}

#[tokio::test]
async fn a_drain_writes_passes_until_the_census_reads_empty() {
    let case = DrainCase::new(vec![2, 0]);

    let outcome = case.drain(&redis_buffer(&case.lists)).await;

    outcome.unwrap();
    assert_eq!(case.events.entries(), ["census 2", "drain", "census 0"]);
}

#[tokio::test]
async fn a_census_that_never_empties_stops_after_twenty_passes() {
    let case = DrainCase::new(vec![1]);

    let outcome = case.drain(&redis_buffer(&case.lists)).await;

    outcome.unwrap();
    let drain_passes = case
        .events
        .entries()
        .iter()
        .filter(|entry| entry.as_str() == "drain")
        .count();
    let censuses = case
        .events
        .entries()
        .iter()
        .filter(|entry| entry.starts_with("census"))
        .count();
    assert_eq!(drain_passes, 20);
    assert_eq!(censuses, 21);
}

#[tokio::test]
async fn a_failing_pass_ends_the_drain_and_still_parks_the_rows() {
    let case = DrainCase::new(vec![2]);
    case.service.fail_drains();
    case.enqueue(&["a", "b"]);

    let outcome = case.drain(&redis_buffer(&case.lists)).await;

    outcome.unwrap_err();
    assert!(case.queue.is_empty());
    assert_eq!(
        case.lists.pushes(),
        [format!("{REDIS_SPEND_LOGS_BUFFER_KEY} 2")]
    );
}

#[tokio::test]
async fn rows_redis_cannot_take_go_back_to_the_memory_head() {
    let case = DrainCase::new(vec![0]);
    case.enqueue(&["a", "b"]);

    let outcome = case.drain(&memory_only_buffer()).await;

    outcome.unwrap();
    let parked: Vec<String> = case
        .queue
        .dequeue(10)
        .into_iter()
        .map(|row| match row.get("request_id") {
            Some(SpendLogValue::Text(request_id)) => request_id.clone(),
            Some(_) | None => String::new(),
        })
        .collect();
    assert_eq!(parked, ["a", "b"]);
    assert!(case.lists.pushes().is_empty());
}

#[tokio::test]
async fn a_running_monitor_refuses_a_second_start_until_the_drain_stops_it() {
    let case = DrainCase::new(vec![0]);
    let service = Arc::clone(&case.service);
    let queue = Arc::clone(&case.queue);

    case.slot
        .start(
            Arc::clone(&service),
            Arc::clone(&queue),
            monitor_settings(Duration::from_secs(2)),
        )
        .unwrap();
    assert!(matches!(
        case.slot
            .start(service, queue, monitor_settings(Duration::from_secs(2))),
        Err(MonitorStartError::AlreadyRunning)
    ));
    case.drain(&memory_only_buffer()).await.unwrap();
    assert!(!case.slot.is_running());
}

// A monitor polling with no wait between passes: only a drain that really joins it keeps its events from growing.
#[tokio::test]
async fn a_stopped_monitor_runs_no_further_pass() {
    let case = DrainCase::new(vec![0]);
    case.slot
        .start(
            Arc::clone(&case.service),
            Arc::clone(&case.queue),
            monitor_settings(Duration::ZERO),
        )
        .unwrap();

    case.drain(&memory_only_buffer()).await.unwrap();
    let stopped = case.events.entries().len();
    yield_now().await;
    yield_now().await;

    assert_eq!(case.events.entries().len(), stopped);
}

#[tokio::test]
async fn a_drain_that_leaves_rows_logs_how_many_are_lost() {
    let case = DrainCase::new(vec![1]);
    let (logs, _capture) = captured_logs();

    let outcome = case.drain(&memory_only_buffer()).await;

    outcome.unwrap();
    assert!(logs.text().contains(&format!(
        "1 spend log rows still queued after {MAX_DRAIN_PASSES} drain passes"
    )));
}

#[tokio::test]
async fn a_drain_that_empties_on_its_last_census_logs_no_lost_rows() {
    let mut census = vec![1; usize::try_from(MAX_DRAIN_PASSES).unwrap()];
    census.push(0);
    let case = DrainCase::new(census);
    let (logs, _capture) = captured_logs();

    let outcome = case.drain(&memory_only_buffer()).await;

    outcome.unwrap();
    assert!(!logs.text().contains("still queued"));
}

// The row the real daily user upsert writes, with every column that table requires.
fn user_spend_row() -> SpendRow {
    SpendRow::from([
        (
            "date".to_owned(),
            SpendColumnValue::Text("2026-01-02".to_owned()),
        ),
        (
            "user_id".to_owned(),
            SpendColumnValue::Text("user-a".to_owned()),
        ),
        (
            "api_key".to_owned(),
            SpendColumnValue::Text("key-1".to_owned()),
        ),
        (
            "model".to_owned(),
            SpendColumnValue::Text("model-1".to_owned()),
        ),
        (
            "custom_llm_provider".to_owned(),
            SpendColumnValue::Text("openai".to_owned()),
        ),
        ("spend".to_owned(), SpendColumnValue::Decimal(Decimal::ONE)),
    ])
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["spend-flush-shutdown-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the database url");
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(3)
        .build()
        .expect("the database pool");
    (fixture, PostgresStore { pool: Some(pool) })
}

// A real statement barrier: the chunk's upsert waits on the table lock, the shutdown cancels before the commit mark, and the row travels back to the queue with nothing written.
#[tokio::test]
async fn a_cancelled_flush_keeps_a_blocked_daily_statement_off_the_database() {
    let (fixture, store) = database().await;
    let queue = Arc::new(
        DailySpendUpdateQueue::new(100, 1000, 100, Arc::new(NoSpendQueueEventSink)).unwrap(),
    );
    queue
        .add_update(DailySpendUpdate::from([(
            "user-a".to_owned(),
            user_spend_row(),
        )]))
        .await
        .unwrap();
    let parts = commit_parts(Arc::new(PostgresDailyChunks {
        store: store.clone(),
    }));
    let commits = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let blocker = store.session().await.unwrap();
    blocker
        .batch_execute("BEGIN; LOCK TABLE \"LiteLLM_DailyUserSpend\" IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    let queued = Arc::clone(&queue);
    let committing = Arc::clone(&parts);
    let tracked = commits.clone();
    let stopping = shutdown.clone();
    tasks.spawn(async move {
        flush_daily_spend_queue(
            &queued,
            DailySpendEntity::User,
            &committing,
            &tracked,
            &stopping,
        )
        .await
    });
    sleep(Duration::from_millis(200)).await;
    shutdown.cancel();
    yield_now().await;
    assert!(tasks.try_join_next().is_none());

    blocker.batch_execute("ROLLBACK").await.unwrap();
    let settled = tasks.join_next().await.unwrap().unwrap();
    assert!(matches!(settled, Err(DailyFlushError::Cancelled { .. })));
    let restored = queue.flush_aggregated_daily_rows().await.unwrap();
    assert_eq!(restored.keys().collect::<Vec<_>>(), ["user-a"]);
    let reader = store.session().await.unwrap();
    let rows = reader
        .query("SELECT spend FROM \"LiteLLM_DailyUserSpend\"", &[])
        .await
        .unwrap();
    assert!(rows.is_empty());

    drop(reader);
    drop(blocker);
    commits.close();
    commits.wait().await;
    if let Some(pool) = store.pool {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}
