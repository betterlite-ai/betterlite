#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::oneshot;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::order::{entity_id_field, ordered_chunk_keys};
use super::postgres::{DailyChunkError, PostgresDailyChunks};
use super::*;
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::session::PostgresStore;
use crate::spend_commit::phase::{
    CommitPhase, CommitPhaseError, CommitPhaseState, CommitTransition, DailyCommitSettlement,
    start_daily_spend_commit,
};
use crate::spend_commit::policy::{SpendFailure, SpendRetryPolicy, SpendUpdateFailure};
use crate::spend_log_storage::rows::writer::{SpendLogFailureEvent, SpendLogFailureSink};
use crate::spend_queues::daily::DailySpendUpdate;
use crate::tool_spend::RetryPause;

#[derive(Debug, thiserror::Error)]
#[error("scripted failure")]
struct ScriptedFailure {
    failure: SpendFailure,
    cancelled: bool,
}

impl ScriptedFailure {
    const fn new(failure: SpendFailure) -> Self {
        Self {
            failure,
            cancelled: false,
        }
    }

    const fn cancelled() -> Self {
        Self {
            failure: SpendFailure::Other,
            cancelled: true,
        }
    }

    const fn cancelled_retryable() -> Self {
        Self {
            failure: SpendFailure::NotSent,
            cancelled: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("plain failure")]
struct PlainFailure;

impl SpendUpdateFailure for PlainFailure {
    fn spend_failure(&self) -> SpendFailure {
        SpendFailure::Other
    }
}

impl DailyChunkFailure for PlainFailure {}

impl SpendUpdateFailure for ScriptedFailure {
    fn spend_failure(&self) -> SpendFailure {
        self.failure
    }
}

impl DailyChunkFailure for ScriptedFailure {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

#[derive(Clone, Copy)]
enum Step {
    Ok,
    Cancelled,
    CancelledRetryable,
    ConnectError,
    Rejected,
    Deadlock,
    Other,
}

fn error_for(step: Step) -> ScriptedFailure {
    match step {
        Step::Ok => ScriptedFailure::new(SpendFailure::Other),
        Step::Cancelled => ScriptedFailure::cancelled(),
        Step::CancelledRetryable => ScriptedFailure::cancelled_retryable(),
        Step::ConnectError => ScriptedFailure::new(SpendFailure::NotSent),
        Step::Rejected => ScriptedFailure::new(SpendFailure::Rejected),
        Step::Deadlock => ScriptedFailure::new(SpendFailure::Deadlock),
        Step::Other => ScriptedFailure::new(SpendFailure::Other),
    }
}

#[derive(Default)]
struct ScriptedWriter {
    script: Mutex<VecDeque<Step>>,
    chunks: Mutex<Vec<Vec<String>>>,
}

impl ScriptedWriter {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            script: Mutex::new(VecDeque::from(steps)),
            chunks: Mutex::new(Vec::new()),
        }
    }

    fn chunks(&self) -> Vec<Vec<String>> {
        self.chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next_step(&self) -> Step {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Step::Ok)
    }
}

impl DailyChunkWriter for ScriptedWriter {
    type Error = ScriptedFailure;

    fn write_chunk<'writer>(
        &'writer self,
        entity: DailySpendEntity,
        rows: &'writer [SpendRow],
        phase: &'writer CommitPhase,
    ) -> ChunkFuture<'writer, ScriptedFailure> {
        Box::pin(async move {
            let id_field = entity_id_field(entity);
            self.chunks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(rows.iter().map(|row| field_text(row, id_field)).collect());
            match self.next_step() {
                Step::Ok => {
                    phase.apply(CommitTransition::Begin).unwrap();
                    phase.apply(CommitTransition::Finish).unwrap();
                    Ok(())
                }
                Step::Cancelled => match phase.apply(CommitTransition::Begin) {
                    Ok(()) => {
                        phase.apply(CommitTransition::Finish).unwrap();
                        Ok(())
                    }
                    Err(_cancelled) => Err(ScriptedFailure::cancelled()),
                },
                step => Err(error_for(step)),
            }
        })
    }
}

struct FixedClock {
    instant: Instant,
}

impl WriteClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH
    }

    fn instant(&self) -> Instant {
        self.instant
    }
}

struct FixedJitter;

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

#[derive(Default)]
struct RecordingPause {
    delays: Mutex<Vec<Duration>>,
}

impl RecordingPause {
    fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
        Box::pin(std::future::ready(()))
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<SpendLogFailureEvent>>,
}

impl RecordingSink {
    fn events(&self) -> Vec<SpendLogFailureEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendLogFailureSink for RecordingSink {
    fn emit(&self, event: SpendLogFailureEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

struct Ports {
    clock: FixedClock,
    jitter: FixedJitter,
    pause: RecordingPause,
    sink: RecordingSink,
}

impl Ports {
    fn new() -> Self {
        Self {
            clock: FixedClock {
                instant: Instant::now(),
            },
            jitter: FixedJitter,
            pause: RecordingPause::default(),
            sink: RecordingSink::default(),
        }
    }

    fn policy(&self, retries: u32) -> SpendRetryPolicy<'_> {
        SpendRetryPolicy {
            retries,
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &self.pause,
            failures: &self.sink,
        }
    }
}

fn field_text(row: &SpendRow, field: &str) -> String {
    match row.get(field) {
        Some(SpendColumnValue::Text(text)) => text.clone(),
        None
        | Some(
            SpendColumnValue::Null
            | SpendColumnValue::Bool(_)
            | SpendColumnValue::Int(_)
            | SpendColumnValue::Decimal(_)
            | SpendColumnValue::Array(_)
            | SpendColumnValue::Object(_),
        ) => String::new(),
    }
}

fn daily_row(date: &str, id_field: &str, id: &str, spend: i64) -> SpendRow {
    let mut row = SpendRow::new();
    row.insert("date".to_owned(), SpendColumnValue::Text(date.to_owned()));
    row.insert(id_field.to_owned(), SpendColumnValue::Text(id.to_owned()));
    row.insert(
        "api_key".to_owned(),
        SpendColumnValue::Text("key-1".to_owned()),
    );
    row.insert(
        "model".to_owned(),
        SpendColumnValue::Text("model-1".to_owned()),
    );
    row.insert(
        "custom_llm_provider".to_owned(),
        SpendColumnValue::Text("openai".to_owned()),
    );
    row.insert(
        "spend".to_owned(),
        SpendColumnValue::Decimal(Decimal::from(spend)),
    );
    row
}

fn pending_rows(count: usize, id_field: &str) -> DailySpendUpdate {
    let mut pending = DailySpendUpdate::new();
    for index in 0..count {
        let key = format!("row-{index:03}");
        let id = format!("user-{index:03}");
        pending.insert(key, daily_row("2026-01-02", id_field, &id, 1));
    }
    pending
}

#[tokio::test]
async fn chunks_take_at_most_a_hundred_sorted_rows() {
    let writer = ScriptedWriter::new(Vec::new());
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(0),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = pending_rows(201, "user_id");
    commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap();
    assert!(pending.is_empty());
    let chunks = writer.chunks();
    assert_eq!(
        chunks.iter().map(Vec::len).collect::<Vec<usize>>(),
        vec![100, 100, 1]
    );
    assert_eq!(chunks[0].first().map(String::as_str), Some("user-000"));
    assert_eq!(chunks[0].last().map(String::as_str), Some("user-099"));
    assert_eq!(chunks[1].first().map(String::as_str), Some("user-100"));
    assert_eq!(chunks[2].first().map(String::as_str), Some("user-200"));
}

#[rstest]
#[case::user(DailySpendEntity::User, "user_id")]
#[case::team(DailySpendEntity::Team, "team_id")]
#[case::org(DailySpendEntity::Org, "organization_id")]
#[case::tag(DailySpendEntity::Tag, "tag")]
#[case::end_user(DailySpendEntity::EndUser, "end_user_id")]
#[case::agent(DailySpendEntity::Agent, "agent_id")]
#[tokio::test]
async fn each_adapter_sorts_by_its_entity_id(
    #[case] entity: DailySpendEntity,
    #[case] id_field: &str,
) {
    assert_eq!(entity_id_field(entity), id_field);
    let writer = ScriptedWriter::new(Vec::new());
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(0),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = DailySpendUpdate::new();
    pending.insert(
        "row-z".to_owned(),
        daily_row("2026-01-02", id_field, "z-entity", 1),
    );
    pending.insert(
        "row-a".to_owned(),
        daily_row("2026-01-02", id_field, "a-entity", 1),
    );
    commit
        .update_daily_spend(entity, &mut pending)
        .await
        .unwrap();
    assert_eq!(writer.chunks(), vec![vec!["a-entity", "z-entity"]]);
}

#[test]
fn missing_sort_values_come_first() {
    let entity = DailySpendEntity::User;
    let mut pending = DailySpendUpdate::new();
    pending.insert(
        "row-dated".to_owned(),
        daily_row("2026-01-02", "user_id", "user-a", 1),
    );
    let mut undated = daily_row("", "user_id", "user-b", 1);
    undated.remove("date");
    pending.insert("row-undated".to_owned(), undated);
    let keys = ordered_chunk_keys(entity, &pending);
    assert_eq!(keys, vec!["row-undated".to_owned(), "row-dated".to_owned()]);
}

#[test]
fn equal_sort_fields_keep_the_pending_key_order() {
    let mut pending = DailySpendUpdate::new();
    let row = daily_row("2026-01-02", "user_id", "user-a", 1);
    pending.insert("row-b".to_owned(), row.clone());
    pending.insert("row-a".to_owned(), row.clone());
    pending.insert("row-c".to_owned(), row);
    let keys = ordered_chunk_keys(DailySpendEntity::User, &pending);
    assert_eq!(
        keys,
        vec!["row-a".to_owned(), "row-b".to_owned(), "row-c".to_owned()]
    );
}

#[tokio::test]
async fn unsafe_failure_drops_its_chunk_and_keeps_the_rest() {
    let writer = ScriptedWriter::new(vec![Step::Ok, Step::Rejected]);
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(0),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = pending_rows(201, "user_id");
    let error = commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap_err();
    assert_eq!(error.spend_failure(), SpendFailure::Rejected);
    assert_eq!(pending.keys().cloned().collect::<Vec<_>>(), vec!["row-200"]);
    assert_eq!(writer.chunks().len(), 2);
    assert_eq!(ports.sink.events().len(), 1);
}

#[tokio::test]
async fn retry_safe_failure_keeps_its_chunk_and_reports_twice() {
    let writer = ScriptedWriter::new(vec![Step::Ok, Step::ConnectError]);
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(0),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = pending_rows(201, "user_id");
    let error = commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap_err();
    assert_eq!(error.spend_failure(), SpendFailure::NotSent);
    assert_eq!(pending.len(), 101);
    assert_eq!(writer.chunks().len(), 2);
    assert_eq!(ports.sink.events().len(), 2);
    assert!(ports.pause.delays().is_empty());
}

#[tokio::test]
async fn exhausted_retryable_failure_retries_once_and_reports_twice() {
    let writer = ScriptedWriter::new(vec![Step::Deadlock, Step::ConnectError]);
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(1),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = pending_rows(1, "user_id");
    let error = commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap_err();
    assert_eq!(error.spend_failure(), SpendFailure::NotSent);
    assert_eq!(pending.len(), 1);
    assert_eq!(writer.chunks().len(), 2);
    assert_eq!(ports.pause.delays(), vec![Duration::from_secs(1)]);
    assert_eq!(ports.sink.events().len(), 2);
}

#[tokio::test]
async fn cancelled_phase_keeps_the_rows_and_skips_the_report() {
    let writer = ScriptedWriter::new(vec![Step::Cancelled]);
    let ports = Ports::new();
    let phase = Arc::new(CommitPhase::new());
    phase.apply(CommitTransition::Cancel).unwrap();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(0),
        phase,
    };
    let mut pending = pending_rows(1, "user_id");
    let error = commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap_err();
    assert!(error.is_cancelled());
    assert_eq!(pending.len(), 1);
    assert!(ports.sink.events().is_empty());
}

#[tokio::test]
async fn a_cancelled_chunk_skips_the_retry_rule() {
    let writer = ScriptedWriter::new(vec![Step::CancelledRetryable]);
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(3),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = pending_rows(1, "user_id");
    let error = commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap_err();
    assert!(error.is_cancelled());
    assert_eq!(writer.chunks().len(), 1);
    assert_eq!(pending.len(), 1);
    assert!(ports.pause.delays().is_empty());
    assert!(ports.sink.events().is_empty());
}

#[test]
fn a_plain_failure_is_not_a_cancellation() {
    assert!(!PlainFailure.is_cancelled());
}

#[test]
fn only_the_cancelled_phase_counts_as_a_cancellation() {
    assert!(DailyChunkError::Phase(CommitPhaseError::Cancelled).is_cancelled());
    assert!(!DailyChunkError::Phase(CommitPhaseError::NotCommitting).is_cancelled());
    assert!(!DailyChunkError::Timeout.is_cancelled());
}

#[test]
fn phase_transitions_are_compare_and_swap() {
    let phase = CommitPhase::new();
    assert_eq!(phase.state(), CommitPhaseState::Waiting);
    phase.apply(CommitTransition::Begin).unwrap();
    assert_eq!(
        phase.apply(CommitTransition::Begin).unwrap_err(),
        CommitPhaseError::AlreadyCommitting
    );
    assert_eq!(
        phase.apply(CommitTransition::Cancel).unwrap_err(),
        CommitPhaseError::AlreadyCommitting
    );
    phase.apply(CommitTransition::Finish).unwrap();
    assert_eq!(
        phase.apply(CommitTransition::Finish).unwrap_err(),
        CommitPhaseError::NotCommitting
    );
    phase.apply(CommitTransition::Cancel).unwrap();
    assert_eq!(phase.state(), CommitPhaseState::Cancelled);
    assert_eq!(
        phase.apply(CommitTransition::Cancel).unwrap_err(),
        CommitPhaseError::Cancelled
    );
    assert_eq!(
        phase.apply(CommitTransition::Begin).unwrap_err(),
        CommitPhaseError::Cancelled
    );
}

#[tokio::test]
async fn shutdown_cancels_a_commit_that_never_reached_its_phase() {
    let commits = TaskTracker::new();
    let (entered, entered_wait) = oneshot::channel();
    let (release, release_wait) = oneshot::channel::<()>();
    let started = start_daily_spend_commit(&commits, move |phase| async move {
        entered.send(()).unwrap();
        let _released = release_wait.await;
        phase.apply(CommitTransition::Begin).unwrap();
    });
    entered_wait.await.unwrap();
    let settlement = started.settle_for_shutdown().await;
    assert!(matches!(settlement, DailyCommitSettlement::Cancelled));
    drop(release);
    commits.close();
    commits.wait().await;
}

#[tokio::test]
async fn shutdown_waits_for_a_commit_that_entered_its_phase() {
    let commits = TaskTracker::new();
    let (entered, entered_wait) = oneshot::channel();
    let (release, release_wait) = oneshot::channel::<()>();
    let started = start_daily_spend_commit(&commits, move |phase| async move {
        phase.apply(CommitTransition::Begin).unwrap();
        entered.send(()).unwrap();
        let _released = release_wait.await;
        phase.apply(CommitTransition::Finish).unwrap();
        7_u32
    });
    let phase = Arc::clone(&started.phase);
    entered_wait.await.unwrap();
    assert_eq!(phase.state(), CommitPhaseState::Committing);
    let mut settlement = Box::pin(started.settle_for_shutdown());
    assert!(
        tokio::time::timeout(Duration::from_millis(10), settlement.as_mut())
            .await
            .is_err()
    );
    release.send(()).unwrap();
    let value = settlement.await;
    assert!(matches!(value, DailyCommitSettlement::Settled(Ok(7))));
    assert_eq!(phase.state(), CommitPhaseState::Waiting);
    commits.close();
    commits.wait().await;
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, PostgresStore, Client) {
    let url = Environment::parse_from(["spend-commit-daily-tests"])
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
        .max_size(2)
        .build()
        .expect("the database pool");
    let store = PostgresStore { pool: Some(pool) };
    let session = store.session().await.expect("the daily spend session");
    (fixture, store, session)
}

async fn close(fixture: PostgresFixture, store: PostgresStore, session: Client) {
    drop(session);
    if let Some(pool) = store.pool {
        pool.close();
    }
    fixture.close_async().await.expect("the fixture teardown");
}

#[tokio::test]
async fn postgres_chunks_apply_and_clear_the_phase() {
    let (fixture, store, session) = database().await;
    let writer = PostgresDailyChunks {
        store: store.clone(),
    };
    let phase = CommitPhase::new();
    let row = daily_row("2026-01-02", "user_id", "user-a", 1);
    writer
        .commit_transaction(DailySpendEntity::User, std::slice::from_ref(&row), &phase)
        .await
        .unwrap();
    assert_eq!(phase.state(), CommitPhaseState::Waiting);
    writer
        .commit_transaction(DailySpendEntity::User, &[row], &phase)
        .await
        .unwrap();
    let spend: f64 = session
        .query_one(
            "SELECT spend FROM \"LiteLLM_DailyUserSpend\" WHERE user_id = $1",
            &[&"user-a"],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(spend.to_bits(), 2.0_f64.to_bits());
    close(fixture, store, session).await;
}

async fn daily_user_count(session: &Client) -> i64 {
    session
        .query_one("SELECT COUNT(*) FROM \"LiteLLM_DailyUserSpend\"", &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn postgres_check_violation_is_rejected_and_dropped() {
    let (fixture, store, session) = database().await;
    session
        .batch_execute(
            "ALTER TABLE \"LiteLLM_DailyUserSpend\" ADD CONSTRAINT daily_user_spend_floor CHECK (spend >= 0)",
        )
        .await
        .unwrap();
    let writer = PostgresDailyChunks {
        store: store.clone(),
    };
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(0),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = DailySpendUpdate::new();
    pending.insert(
        "row-a".to_owned(),
        daily_row("2026-01-02", "user_id", "user-a", 1),
    );
    pending.insert(
        "row-b".to_owned(),
        daily_row("2026-01-02", "user_id", "user-b", -5),
    );
    let error = commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap_err();
    assert_eq!(error.spend_failure(), SpendFailure::Rejected);
    assert!(pending.is_empty());
    assert_eq!(ports.sink.events().len(), 1);
    assert_eq!(daily_user_count(&session).await, 0);
    close(fixture, store, session).await;
}

#[tokio::test]
async fn no_database_connection_is_retried_and_retained() {
    let writer = PostgresDailyChunks {
        store: PostgresStore { pool: None },
    };
    let ports = Ports::new();
    let commit = DailyChunkCommit {
        writer: &writer,
        policy: ports.policy(1),
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = pending_rows(1, "user_id");
    let error: DailyChunkError = commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
        .unwrap_err();
    assert_eq!(error.spend_failure(), SpendFailure::NotSent);
    assert_eq!(pending.len(), 1);
    assert_eq!(ports.pause.delays(), vec![Duration::from_secs(1)]);
    assert_eq!(ports.sink.events().len(), 2);
}
