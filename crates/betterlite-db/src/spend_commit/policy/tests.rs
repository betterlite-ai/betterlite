#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_test_ports::refused_port::RefusedPort;
use chrono::{DateTime, Utc};
use deadpool_postgres::{PoolError, TimeoutType};
use rstest::rstest;
use rust_decimal::Decimal;
use tokio_postgres::error::SqlState;

use super::*;
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::diagnostics::{DriverError, PoolFailure};
use crate::spend_commit::tables::{SpendTable, SpendTableTransactions, spend_tables_left_to_send};
use crate::spend_commit::transaction::{SPEND_TRANSACTION_TIMEOUT, SpendDeadline};
use crate::tool_spend::RetryPause;

#[derive(Debug, thiserror::Error)]
#[error("scripted failure")]
struct ScriptedFailure {
    failure: SpendFailure,
}

impl SpendUpdateFailure for ScriptedFailure {
    fn spend_failure(&self) -> SpendFailure {
        self.failure
    }
}

#[derive(Debug, thiserror::Error)]
#[error("outer")]
struct OuterFailure(#[source] InnerFailure);

#[derive(Debug, thiserror::Error)]
#[error("inner")]
struct InnerFailure;

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

struct FixedJitter {
    nanos: u64,
}

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        if upper_exclusive_nanos == 0 {
            return 0;
        }
        self.nanos.min(upper_exclusive_nanos.saturating_sub(1))
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
            jitter: FixedJitter { nanos: 0 },
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

#[rstest]
#[case::not_sent(SpendFailure::NotSent, true, true)]
#[case::ambiguous(SpendFailure::Ambiguous, false, false)]
#[case::deadlock(SpendFailure::Deadlock, true, true)]
#[case::rejected(SpendFailure::Rejected, false, false)]
#[case::other(SpendFailure::Other, true, false)]
fn failure_classes(
    #[case] failure: SpendFailure,
    #[case] requeue_safe: bool,
    #[case] retryable: bool,
) {
    assert_eq!(failure.is_requeue_safe(), requeue_safe);
    assert_eq!(failure.is_retryable(), retryable);
}

#[rstest]
#[case::check_violation("23514", SpendFailure::Rejected)]
#[case::unique_violation("23505", SpendFailure::Rejected)]
#[case::invalid_text("22021", SpendFailure::Rejected)]
#[case::deadlock("40P01", SpendFailure::Deadlock)]
#[case::serialization_failure("40001", SpendFailure::Deadlock)]
#[case::undefined_table("42P01", SpendFailure::Other)]
#[case::internal_error("XX000", SpendFailure::Other)]
fn sqlstate_classes(#[case] code: &str, #[case] expected: SpendFailure) {
    assert_eq!(failure_for_sqlstate(&SqlState::from_code(code)), expected);
}

#[test]
fn classify_no_database_as_not_sent() {
    let error = DbError::<Infallible>::NoDbConnected;
    assert_eq!(classify_spend_failure(&error), SpendFailure::NotSent);
}

#[test]
fn classify_pool_wait_timeout_as_not_sent() {
    let error = DbError::<Infallible>::Pool {
        source: PoolFailure {
            driver: PoolError::Timeout(TimeoutType::Wait),
        },
    };
    assert_eq!(classify_spend_failure(&error), SpendFailure::NotSent);
}

#[test]
fn classify_closed_pool_as_not_sent() {
    let error = DbError::<Infallible>::Pool {
        source: PoolFailure {
            driver: PoolError::Closed,
        },
    };
    assert_eq!(classify_spend_failure(&error), SpendFailure::NotSent);
}

#[test]
fn classify_pool_hook_failure_as_other() {
    let error = DbError::<Infallible>::Pool {
        source: PoolFailure {
            driver: PoolError::NoRuntimeSpecified,
        },
    };
    assert_eq!(classify_spend_failure(&error), SpendFailure::Other);
}

#[test]
fn classify_create_null_as_other() {
    let error = DbError::<Infallible>::CreateNull;
    assert_eq!(classify_spend_failure(&error), SpendFailure::Other);
}

async fn refused_connection_error() -> tokio_postgres::Error {
    let refused = RefusedPort::bind().unwrap();
    let Err(error) = tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={} user=missing",
            refused.address.port()
        ),
        tokio_postgres::NoTls,
    )
    .await
    else {
        panic!("connection unexpectedly succeeded");
    };
    error
}

#[tokio::test]
async fn refused_pool_connection_is_not_sent() {
    let value = DbError::<Infallible>::Pool {
        source: PoolFailure {
            driver: PoolError::Backend(refused_connection_error().await),
        },
    };
    assert_eq!(classify_spend_failure(&value), SpendFailure::NotSent);
}

#[tokio::test]
async fn backend_transport_failure_is_ambiguous() {
    let value = DbError::<Infallible>::Backend {
        source: DriverError {
            driver: refused_connection_error().await,
        },
    };
    assert_eq!(classify_spend_failure(&value), SpendFailure::Ambiguous);
}

#[tokio::test]
async fn connection_sqlstate_is_not_sent() {
    let value = DbError::<Infallible>::SqlState {
        code: SqlState::CONNECTION_EXCEPTION,
        constraint: None,
        source: DriverError {
            driver: refused_connection_error().await,
        },
    };
    assert_eq!(classify_spend_failure(&value), SpendFailure::NotSent);
}

#[tokio::test]
async fn retryable_failure_stops_after_the_last_attempt_and_reports() {
    let ports = Ports::new();
    let policy = ports.policy(2);
    let failure = ScriptedFailure {
        failure: SpendFailure::NotSent,
    };
    let started = ports.clock.instant;
    let outcome = handle_spend_update_failure(&policy, &failure, 2, started).await;
    assert_eq!(outcome, SpendRetry::Stop);
    assert_eq!(ports.sink.events().len(), 1);
    assert!(ports.pause.delays().is_empty());
}

#[tokio::test]
async fn non_retryable_failure_reports_without_retrying() {
    let ports = Ports::new();
    let policy = ports.policy(3);
    let failure = ScriptedFailure {
        failure: SpendFailure::Rejected,
    };
    let started = ports.clock.instant;
    let outcome = handle_spend_update_failure(&policy, &failure, 0, started).await;
    assert_eq!(outcome, SpendRetry::Stop);
    assert_eq!(ports.sink.events().len(), 1);
    assert!(ports.pause.delays().is_empty());
}

#[tokio::test]
async fn retryable_failure_sleeps_with_the_doubled_base() {
    let ports = Ports::new();
    let policy = ports.policy(3);
    let failure = ScriptedFailure {
        failure: SpendFailure::Deadlock,
    };
    let started = ports.clock.instant;
    let first = handle_spend_update_failure(&policy, &failure, 0, started).await;
    let second = handle_spend_update_failure(&policy, &failure, 1, started).await;
    let third = handle_spend_update_failure(&policy, &failure, 2, started).await;
    assert_eq!(first, SpendRetry::Retry);
    assert_eq!(second, SpendRetry::Retry);
    assert_eq!(third, SpendRetry::Retry);
    assert_eq!(
        ports.pause.delays(),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
    assert!(ports.sink.events().is_empty());
}

#[test]
fn retry_delay_covers_the_upper_second() {
    let jitter = FixedJitter { nanos: u64::MAX };
    assert_eq!(retry_delay(&jitter, 0), Duration::from_nanos(1_999_999_999));
    assert_eq!(retry_delay(&jitter, 1), Duration::from_nanos(3_999_999_999));
}

#[test]
fn report_keeps_the_cause_chain() {
    let sink = RecordingSink::default();
    let error = OuterFailure(InnerFailure);
    report_failed_update_spend(&sink, &error, Duration::from_millis(25));
    let events = sink.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].call_type, UPDATE_SPEND_CALL_TYPE);
    assert_eq!(events[0].duration, Duration::from_millis(25));
    assert_eq!(
        events[0].diagnostic,
        format!("{NON_BLOCKING_PREFIX} outer\nCaused by: inner")
    );
}

#[tokio::test]
async fn deadline_reports_a_timeout() {
    let deadline = SpendDeadline {
        duration: Duration::from_millis(5),
    };
    let pending = std::future::pending::<Result<(), Infallible>>();
    let outcome = deadline.run(pending).await;
    let SpendTransactionError::Timeout { seconds } = outcome.unwrap_err() else {
        panic!("expected a timed out transaction");
    };
    assert_eq!(seconds, 0);
}

#[tokio::test]
async fn deadline_passes_work_errors_through() {
    let deadline = SpendDeadline {
        duration: Duration::from_secs(1),
    };
    let outcome = deadline
        .run(async { Err::<(), &'static str>("broken") })
        .await;
    assert!(matches!(
        outcome,
        Err(SpendTransactionError::Work("broken"))
    ));
    assert_eq!(SpendDeadline::SOURCE.duration, SPEND_TRANSACTION_TIMEOUT);
}

fn increments(rows: &[(&str, i64)]) -> BTreeMap<String, Decimal> {
    rows.iter()
        .map(|(key, amount)| ((*key).to_owned(), Decimal::from(*amount)))
        .collect()
}

fn table_map(tables: &[(SpendTable, Option<&[(&str, i64)]>)]) -> SpendTableTransactions {
    tables
        .iter()
        .map(|(table, rows)| {
            let value = rows.map(increments);
            (*table, value)
        })
        .collect()
}

#[test]
fn table_order_matches_the_source() {
    let names: Vec<&str> = SpendTable::COMMIT_ORDER
        .iter()
        .map(|table| table.name())
        .collect();
    assert_eq!(
        names,
        vec![
            "user_list_transactions",
            "end_user_list_transactions",
            "key_list_transactions",
            "team_list_transactions",
            "team_member_list_transactions",
            "org_list_transactions",
            "org_member_list_transactions",
            "project_list_transactions",
            "tag_list_transactions",
            "model_access_group_list_transactions",
            "agent_list_transactions",
        ]
    );
}

#[test]
fn safe_failure_keeps_the_table_in_flight() {
    let transactions = table_map(&[
        (SpendTable::UserList, Some(&[("user-1", 3)])),
        (SpendTable::EndUserList, Some(&[("end-1", 4)])),
    ]);
    let remaining =
        spend_tables_left_to_send(&transactions, &[SpendTable::UserList], SpendFailure::Other)
            .unwrap();
    assert_eq!(remaining.get(&SpendTable::UserList), Some(&None));
    assert_eq!(
        remaining.get(&SpendTable::EndUserList),
        Some(&Some(increments(&[("end-1", 4)])))
    );
}

#[test]
fn unsafe_failure_drops_the_table_in_flight() {
    let transactions = table_map(&[
        (SpendTable::UserList, Some(&[("user-1", 3)])),
        (SpendTable::EndUserList, Some(&[("end-1", 4)])),
        (SpendTable::KeyList, Some(&[("key-1", 5)])),
    ]);
    let remaining = spend_tables_left_to_send(
        &transactions,
        &[SpendTable::UserList],
        SpendFailure::Rejected,
    )
    .unwrap();
    assert_eq!(remaining.get(&SpendTable::UserList), Some(&None));
    assert_eq!(remaining.get(&SpendTable::EndUserList), Some(&None));
    assert_eq!(
        remaining.get(&SpendTable::KeyList),
        Some(&Some(increments(&[("key-1", 5)])))
    );
}

#[test]
fn all_tables_committed_leaves_nothing() {
    let transactions = table_map(&[(SpendTable::AgentList, Some(&[("agent-1", 2)]))]);
    assert_eq!(
        spend_tables_left_to_send(
            &transactions,
            &SpendTable::COMMIT_ORDER,
            SpendFailure::Rejected
        ),
        None
    );
}

#[test]
fn empty_increments_leave_nothing() {
    let transactions = table_map(&[(SpendTable::UserList, None)]);
    assert_eq!(
        spend_tables_left_to_send(&transactions, &[], SpendFailure::NotSent),
        None
    );
}
