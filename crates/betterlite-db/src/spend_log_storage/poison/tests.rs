#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::postgres_store::errors::backend;
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::events::NoRedisBufferEventSink;
use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_batches::StatementLimits;
use crate::spend_log_storage::postgres::{PostgresSpendLogsWriter, SpendLogStoreError};
use crate::spend_log_storage::rows::SpendLogQueue;
use crate::spend_log_storage::rows::writer::{FlushPolicy, NoSpendLogFailureSink, SpendLogFlush};
use crate::spend_log_storage::tests::support::{
    NoClient, RecordingPause, ScriptedFailure, SpendLogCase, SteppingClock, ids, ordinary_row,
    text, unreachable_store,
};
use crate::spend_log_storage::{SpendLogFuture, SpendLogWriteError, SpendLogsWriter};

use super::{MAX_ISOLATION_FAILURES, create_logs_isolating_poison};

#[derive(Default)]
struct ScriptedWriter {
    poison: BTreeSet<String>,
    failure: Option<ScriptedFailure>,
    attempts: Mutex<Vec<Vec<String>>>,
    persisted: Mutex<Vec<String>>,
}

impl ScriptedWriter {
    fn new(poison: BTreeSet<String>, failure: Option<ScriptedFailure>) -> Self {
        Self {
            poison,
            failure,
            attempts: Mutex::new(Vec::new()),
            persisted: Mutex::new(Vec::new()),
        }
    }

    fn attempt(&self, rows: &[SpendLogRow]) -> Result<usize, ScriptedFailure> {
        let attempted = ids(rows);
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(attempted.clone());
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        if attempted.iter().any(|id| self.poison.contains(id)) {
            return Err(ScriptedFailure::DATA);
        }
        self.persisted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(attempted);
        Ok(rows.len())
    }

    fn attempted(&self) -> Vec<String> {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|batch| batch.join(","))
            .collect()
    }

    fn attempted_count(&self) -> usize {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn persisted(&self) -> Vec<String> {
        self.persisted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendLogsWriter for ScriptedWriter {
    type Error = ScriptedFailure;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, ScriptedFailure>> {
        Box::pin(async move { self.attempt(rows) })
    }
}

struct CountingWriter {
    inner: PostgresSpendLogsWriter,
    attempts: AtomicUsize,
}

impl SpendLogsWriter for CountingWriter {
    type Error = SpendLogStoreError;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, SpendLogStoreError>> {
        Box::pin(async move {
            let _attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            self.inner.insert_rows(rows).await
        })
    }
}

#[tokio::test]
async fn a_poison_row_is_isolated_and_its_neighbours_persist() {
    let writer = ScriptedWriter::new(BTreeSet::from(["b".to_owned()]), None);
    let rows = [ordinary_row("a"), ordinary_row("b"), ordinary_row("c")];
    let budget = create_logs_isolating_poison(&writer, &rows, MAX_ISOLATION_FAILURES)
        .await
        .expect("the isolation settles");
    assert_eq!(writer.attempted(), ["a,b,c", "a", "b,c", "b", "c"]);
    assert_eq!(writer.persisted(), ["a", "c"]);
    assert_eq!(budget, 253);
}

#[tokio::test]
async fn an_outage_is_never_bisected() {
    let writer = ScriptedWriter::new(BTreeSet::new(), Some(ScriptedFailure::OUTAGE));
    let rows = [
        ordinary_row("a"),
        ordinary_row("b"),
        ordinary_row("c"),
        ordinary_row("d"),
    ];
    let error = create_logs_isolating_poison(&writer, &rows, 256)
        .await
        .expect_err("the outage propagates");
    assert_eq!(writer.attempted_count(), 1);
    assert!(error.is_service_unavailable());
    assert!(error.is_transient());
    assert!(!error.is_data_rejection());
}

#[rstest::rstest]
#[case::an_outage(ScriptedFailure::MISLABELLED_OUTAGE)]
#[case::a_deadlock(ScriptedFailure::MISLABELLED_DEADLOCK)]
#[tokio::test]
async fn a_data_error_that_is_an_outage_is_not_bisected(#[case] failure: ScriptedFailure) {
    let writer = ScriptedWriter::new(BTreeSet::new(), Some(failure));
    let rows = [ordinary_row("a"), ordinary_row("b")];
    let error = create_logs_isolating_poison(&writer, &rows, 256)
        .await
        .expect_err("the outage propagates");
    assert_eq!(writer.attempted_count(), 1);
    assert!(error.is_database());
    assert!(writer.persisted().is_empty());
}

#[tokio::test]
async fn a_clean_statement_keeps_the_whole_budget() {
    let writer = ScriptedWriter::default();
    let rows = [ordinary_row("a"), ordinary_row("b")];
    let budget = create_logs_isolating_poison(&writer, &rows, 7)
        .await
        .expect("the statement lands");
    assert_eq!(budget, 7);
    assert_eq!(writer.attempted(), ["a,b"]);
}

#[tokio::test]
async fn a_singleton_poison_row_costs_one_attempt() {
    let writer = ScriptedWriter::new(BTreeSet::from(["b".to_owned()]), None);
    let budget = create_logs_isolating_poison(&writer, &[ordinary_row("b")], 5)
        .await
        .expect("the singleton is dropped");
    assert_eq!(budget, 4);
    assert_eq!(writer.attempted(), ["b"]);
    assert!(writer.persisted().is_empty());
}

#[tokio::test]
async fn an_exhausted_budget_drops_the_remainder_without_more_attempts() {
    let writer = ScriptedWriter::new(BTreeSet::from(["a".to_owned()]), None);
    let rows = [
        ordinary_row("a"),
        ordinary_row("b"),
        ordinary_row("c"),
        ordinary_row("d"),
    ];
    let budget = create_logs_isolating_poison(&writer, &rows, 0)
        .await
        .expect("the remainder is dropped");
    assert_eq!(budget, 0);
    assert_eq!(writer.attempted(), ["a,b,c,d"]);
    assert!(writer.persisted().is_empty());
}

#[tokio::test]
async fn a_healthy_group_after_an_exhausted_group_still_persists() {
    let poison: BTreeSet<String> = (0..1000).map(|index| format!("poison-{index}")).collect();
    let mut rows: Vec<SpendLogRow> = poison.iter().map(|id| ordinary_row(id)).collect();
    rows.push(ordinary_row("healthy"));
    let writer = ScriptedWriter::new(poison, None);
    let queue = SpendLogQueue::new(usize::MAX);
    let redis = RedisUpdateBuffer {
        lists: None,
        enabled: false,
        dequeue_count: 10,
        events: Arc::new(NoRedisBufferEventSink),
    };
    let pause = RecordingPause::default();
    let clock = SteppingClock {
        start: Instant::now(),
        steps: AtomicU32::new(0),
    };
    let flush = SpendLogFlush {
        writer: &writer,
        external: None::<&NoClient>,
        queue: &queue,
        redis: &redis,
        failures: &NoSpendLogFailureSink,
        pause: &pause,
        clock: &clock,
        policy: FlushPolicy {
            retries: 0,
            limits: StatementLimits {
                max_bytes: usize::MAX,
                max_rows: 1000,
            },
            spend_logs_url: None,
        },
    };
    flush.run(Some(&rows)).await.expect("the pass settles");
    assert_eq!(writer.attempted_count(), 257);
    assert_eq!(writer.persisted(), ["healthy"]);
    assert!(queue.is_empty());
}

#[tokio::test]
async fn a_data_rejection_is_isolated_and_the_good_rows_persist() {
    let case = SpendLogCase::new().await;
    let table = PostgresSpendLogsWriter {
        store: case.store.clone(),
    };
    let mut poison = ordinary_row("poison-1");
    let _replaced = poison.insert("model".to_owned(), text("gpt-4o\u{0}"));
    let rows = [ordinary_row("good-1"), poison, ordinary_row("good-2")];
    let budget = create_logs_isolating_poison(&table, &rows, MAX_ISOLATION_FAILURES)
        .await
        .expect("the isolation settles");
    assert_eq!(budget, 253);
    let persisted: Vec<String> = case
        .rows()
        .await
        .into_iter()
        .map(|(request_id, _, _, _)| request_id)
        .collect();
    assert_eq!(persisted, ["good-1", "good-2"]);
    case.close().await;
}

#[tokio::test]
async fn an_outage_is_not_mistaken_for_a_poison_row() {
    let writer = CountingWriter {
        inner: PostgresSpendLogsWriter {
            store: unreachable_store(),
        },
        attempts: AtomicUsize::new(0),
    };
    let rows = [ordinary_row("a"), ordinary_row("b")];
    let error = create_logs_isolating_poison(&writer, &rows, 256)
        .await
        .expect_err("the outage propagates");
    assert_eq!(writer.attempts.load(Ordering::SeqCst), 1);
    assert!(error.is_service_unavailable());
    assert!(error.is_transient());
    assert!(!error.is_data_rejection());
    assert!(error.is_database());
}

#[tokio::test]
async fn a_plain_database_error_is_not_bisected() {
    let writer = ScriptedWriter::new(BTreeSet::new(), Some(ScriptedFailure::DATABASE_ERROR));
    let rows = [ordinary_row("a"), ordinary_row("b")];
    let error = create_logs_isolating_poison(&writer, &rows, 256)
        .await
        .expect_err("the database error propagates");
    assert_eq!(writer.attempted_count(), 1);
    assert!(writer.persisted().is_empty());
    assert!(error.is_database());
    assert!(!error.is_transient());
}

#[tokio::test]
async fn native_sqlstate_classification_follows_the_sqlstate_class() {
    let case = SpendLogCase::new().await;
    let rejection = case
        .client
        .batch_execute(
            "DO $$ BEGIN RAISE EXCEPTION 'invalid byte sequence' USING ERRCODE = '22021'; END $$",
        )
        .await
        .expect_err("the statement raises a data rejection");
    let data_failure = SpendLogStoreError::Database(backend(rejection));
    assert!(data_failure.is_data_rejection());
    assert!(!data_failure.is_transient());
    assert!(!data_failure.is_service_unavailable());
    assert!(!data_failure.is_deadlock());
    assert!(data_failure.is_database());
    let deadlock = case
        .client
        .batch_execute(
            "DO $$ BEGIN RAISE EXCEPTION 'deadlock detected' USING ERRCODE = '40P01'; END $$",
        )
        .await
        .expect_err("the statement raises a deadlock");
    let deadlock_failure = SpendLogStoreError::Database(backend(deadlock));
    assert!(deadlock_failure.is_deadlock());
    assert!(deadlock_failure.is_transient());
    assert!(!deadlock_failure.is_data_rejection());
    assert!(!deadlock_failure.is_service_unavailable());
    assert!(deadlock_failure.is_database());
    case.close().await;
}

#[test]
fn a_row_encoding_failure_is_not_a_database_error() {
    let error = SpendLogStoreError::Json(
        serde_json::from_str::<i32>("not json").expect_err("the text is not a number"),
    );
    assert!(!error.is_database());
    assert!(!error.is_transient());
    assert!(!error.is_data_rejection());
    assert!(!error.is_service_unavailable());
    assert!(!error.is_deadlock());
}
