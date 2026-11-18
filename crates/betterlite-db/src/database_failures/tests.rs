#![cfg(test)]

use std::future::pending;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use tokio::sync::Notify;
use tokio::sync::Semaphore;
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{sleep, timeout};
use tokio_postgres::{Config, NoTls};
use tokio_util::task::TaskTracker;

use crate::database_faults::ErrorType;

use super::deadline::{
    DbLookupDeadlineExceeded, DbLookupError, LookupRequest, bounded_db_lookup,
    consume_abandoned_lookup, deadline_text,
};
use super::gate::{DbLookupStallTracker, LookupClock, LoopBoundSemaphore, SystemClock};

use self::support::{
    RecordingResolver, RecordingSink, assert_deadline, capture_logs, database_error, drain,
    metric_call, request, run_metric, stall_tracker, successes,
};

mod metrics;
mod support;

const WINDOW: f64 = 30.0;
const PROBE_TABLE: &str = r#"
CREATE TABLE "LiteLLM_DbLookupProbe" (
    id text PRIMARY KEY,
    note text NOT NULL
);
"#;

fn fake_tracker() -> DbLookupStallTracker<FakeClock> {
    DbLookupStallTracker {
        clock: FakeClock::new(),
        last_hit: Mutex::new(None),
    }
}

async fn delayed_lookup() -> Result<i32, DbLookupError> {
    sleep(Duration::from_millis(50)).await;
    Ok(7_i32)
}

struct FakeClock {
    base: Instant,
    offset: AtomicU64,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: AtomicU64::new(0),
        }
    }

    fn advance(&self, step: Duration) {
        let nanos = u64::try_from(step.as_nanos()).expect("the step fits in u64");
        self.offset.fetch_add(nanos, Ordering::SeqCst);
    }
}

impl LookupClock for FakeClock {
    fn now(&self) -> Instant {
        let nanos = self.offset.load(Ordering::SeqCst);
        self.base
            .checked_add(Duration::from_nanos(nanos))
            .expect("the fake clock stays in range")
    }
}

struct NotifyOnDrop(Arc<Notify>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

#[test]
fn semaphore_runtime_recreation() {
    let gate = LoopBoundSemaphore {
        value: 2,
        slot: Mutex::new(None),
    };
    assert!(gate.current().is_none());
    let first_runtime = Builder::new_current_thread()
        .build()
        .expect("the first runtime builds");
    let first = first_runtime.block_on(async { gate.current().expect("a runtime is running") });
    let same = first_runtime.block_on(async { gate.current().expect("a runtime is running") });
    assert!(Arc::ptr_eq(&first, &same));
    let _held = Arc::clone(&first)
        .try_acquire_owned()
        .expect("the gate has two permits");
    assert_eq!(first.available_permits(), 1);
    drop(first_runtime);
    let second_runtime = Builder::new_current_thread()
        .build()
        .expect("the second runtime builds");
    let second = second_runtime.block_on(async { gate.current().expect("a runtime is running") });
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(second.available_permits(), 2);
}

#[tokio::test]
async fn bounded_lookup_does_not_acquire_gate() {
    let gate = LoopBoundSemaphore {
        value: 1,
        slot: Mutex::new(None),
    };
    let semaphore = gate.current().expect("a runtime is running");
    let _held = Arc::clone(&semaphore)
        .try_acquire_owned()
        .expect("the gate is open");
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let lookup_request = request("policy", Some(0.1_f64), 10.0, &tracker);
    let output = bounded_db_lookup(
        async { Ok::<i32, DbLookupError>(7_i32) },
        lookup_request,
        &tasks,
    )
    .await
    .expect("the lookup never takes the gate");
    assert_eq!(output, 7_i32);
    assert_eq!(semaphore.available_permits(), 0);
}

#[tokio::test]
async fn system_clock_advances() {
    let first = SystemClock.now();
    sleep(Duration::from_millis(1)).await;
    assert!(SystemClock.now() > first);
}

#[test]
fn stall_initially_absent() {
    let tracker = fake_tracker();
    assert!(!tracker.stalled_within(0.0));
    assert!(!tracker.stalled_within(WINDOW));
    assert!(!tracker.stalled_within(f64::INFINITY));
}

#[test]
fn stall_window_strict_boundary() {
    let tracker = fake_tracker();
    tracker.record_hit();
    assert!(tracker.stalled_within(WINDOW));
    tracker.clock.advance(Duration::from_millis(29_999));
    assert!(tracker.stalled_within(WINDOW));
    tracker.clock.advance(Duration::from_millis(1));
    assert!(!tracker.stalled_within(WINDOW));
    tracker.clock.advance(Duration::from_millis(1));
    assert!(!tracker.stalled_within(WINDOW));
    assert!(!tracker.stalled_within(0.0));
}

#[tokio::test]
async fn success_does_not_clear_stall() {
    let tasks = TaskTracker::new();
    let tracker = fake_tracker();
    tracker.record_hit();
    let lookup_request = request("cache", Some(0.1_f64), 10.0, &tracker);
    let output = bounded_db_lookup(
        async { Ok::<i32, DbLookupError>(7_i32) },
        lookup_request,
        &tasks,
    )
    .await
    .expect("the lookup succeeds");
    assert_eq!(output, 7_i32);
    assert!(tracker.stalled_within(WINDOW));
    tracker.clock.advance(Duration::from_secs(30));
    assert!(!tracker.stalled_within(WINDOW));
}

#[tokio::test(start_paused = true)]
async fn explicit_deadline_over_default() {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let short = request("work", Some(0.0_f64), 30.0, &tracker);
    let error = bounded_db_lookup(pending::<Result<i32, DbLookupError>>(), short, &tasks)
        .await
        .expect_err("the explicit zero deadline wins over the default");
    assert_deadline(&error, "work", 0.0);
    let generous = request("work", Some(30.0_f64), 0.0, &tracker);
    let output = bounded_db_lookup(delayed_lookup(), generous, &tasks)
        .await
        .expect("the explicit budget outlives the default");
    assert_eq!(output, 7_i32);
    let fallback = request("work", None, 0.02, &tracker);
    let default_expiry =
        bounded_db_lookup(pending::<Result<i32, DbLookupError>>(), fallback, &tasks)
            .await
            .expect_err("the configured default bounds the lookup");
    assert_deadline(&default_expiry, "work", 0.02);
    drain(&tasks).await;
}

#[tokio::test]
async fn zero_negative_explicit_deadline() {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let zero = request("zero", Some(0.0_f64), 10.0, &tracker);
    let expired_at_zero = bounded_db_lookup(pending::<Result<i32, DbLookupError>>(), zero, &tasks)
        .await
        .expect_err("a zero deadline expires at once");
    assert_deadline(&expired_at_zero, "zero", 0.0);
    assert_eq!(
        expired_at_zero.to_string(),
        "zero lookup did not answer within 0s"
    );
    let negative = request("negative", Some(-1.0_f64), 10.0, &tracker);
    let expired_negative = timeout(
        Duration::from_secs(2),
        bounded_db_lookup(pending::<Result<i32, DbLookupError>>(), negative, &tasks),
    )
    .await
    .expect("a negative deadline does not wait forever")
    .expect_err("a negative deadline expires at once");
    assert_deadline(&expired_negative, "negative", -1.0);
    assert_eq!(
        expired_negative.to_string(),
        "negative lookup did not answer within -1s"
    );
    assert!(tracker.stalled_within(f64::INFINITY));
    drain(&tasks).await;
}

#[rstest]
#[case::nan(f64::NAN)]
#[case::positive_infinity(f64::INFINITY)]
#[case::huge(1e300_f64)]
#[tokio::test]
async fn nonfinite_explicit_deadline_never_expires(#[case] deadline_seconds: f64) {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let lookup_request = request("work", Some(deadline_seconds), 0.0, &tracker);
    let output = bounded_db_lookup(
        async { Ok::<i32, DbLookupError>(7_i32) },
        lookup_request,
        &tasks,
    )
    .await
    .expect("an unrepresentable deadline waits for the lookup");
    assert_eq!(output, 7_i32);
    assert!(!tracker.stalled_within(f64::INFINITY));
}

#[tokio::test(start_paused = true)]
async fn timeout_fields_and_exact_message() {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let lookup_request = request("user_api_key_cache", Some(2.5_f64), 10.0, &tracker);
    let error = bounded_db_lookup(
        pending::<Result<i32, DbLookupError>>(),
        lookup_request,
        &tasks,
    )
    .await
    .expect_err("the pending lookup cannot answer");
    let DbLookupError::Deadline(exceeded) = &error else {
        panic!("the lookup must fail with a deadline, got {error:?}");
    };
    assert_eq!(exceeded.lookup, "user_api_key_cache");
    assert_eq!(exceeded.deadline_seconds.to_bits(), 2.5_f64.to_bits());
    assert!(exceeded.cause.is_none());
    assert_eq!(
        error.to_string(),
        "user_api_key_cache lookup did not answer within 2.5s"
    );
    assert!(tracker.stalled_within(f64::INFINITY));
    drain(&tasks).await;
}

#[rstest]
#[case::zero(0.0_f64, "0")]
#[case::negative_zero(-0.0_f64, "-0")]
#[case::one(1.0_f64, "1")]
#[case::two_point_five(2.5_f64, "2.5")]
#[case::ten(10.0_f64, "10")]
#[case::point_one(0.1_f64, "0.1")]
#[case::point_zero_five(0.05_f64, "0.05")]
#[case::fixed_edge(0.0001_f64, "0.0001")]
#[case::scientific_edge(0.00001_f64, "1e-05")]
#[case::tiny(1e-7_f64, "1e-07")]
#[case::million(1_000_000.0_f64, "1e+06")]
#[case::six_digits(123_456.0_f64, "123456")]
#[case::seven_digits(1_234_567.0_f64, "1.23457e+06")]
#[case::large(123_456_789.0_f64, "1.23457e+08")]
#[case::twenty_five(25.0_f64, "25")]
#[case::rounding_tail(0.300_000_000_000_000_04_f64, "0.3")]
#[case::rounding_up(1.799_999_999_999_999_8_f64, "1.8")]
#[case::nan(f64::NAN, "nan")]
#[case::inf(f64::INFINITY, "inf")]
#[case::negative_inf(f64::NEG_INFINITY, "-inf")]
fn timeout_message_uses_python_general_format(#[case] value: f64, #[case] expected: &str) {
    assert_eq!(deadline_text(value), expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_resistant_lookup_returns_deadline() {
    let tasks = TaskTracker::new();
    let blocking = TaskTracker::new();
    let tracker = stall_tracker();
    let (release, blocker) = sync_channel::<()>(1);
    let finished = Arc::new(AtomicBool::new(false));
    let lookup_finished = Arc::clone(&finished);
    let join = blocking.spawn_blocking(move || {
        blocker.recv().expect("the test releases the lookup");
        lookup_finished.store(true, Ordering::SeqCst);
    });
    let lookup = async move {
        join.await.expect("the blocking lookup joins");
        Ok::<i32, DbLookupError>(7_i32)
    };
    let lookup_request = request("slow", Some(0.02_f64), 10.0, &tracker);
    let result = bounded_db_lookup(lookup, lookup_request, &tasks).await;
    let answered_early = finished.load(Ordering::SeqCst);
    release.send(()).expect("the lookup is still waiting");
    blocking.close();
    blocking.wait().await;
    assert!(!answered_early, "the blocked lookup cannot answer in time");
    let error = result.expect_err("the blocked lookup cannot answer in time");
    assert_deadline(&error, "slow", 0.02);
    assert!(finished.load(Ordering::SeqCst));
    drain(&tasks).await;
}

#[tokio::test]
async fn abandoned_failure_consumed() {
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    let blocking = TaskTracker::new();
    let (release, blocker) = sync_channel::<()>(1);
    let produced = Arc::new(AtomicBool::new(false));
    let lookup_produced = Arc::clone(&produced);
    let join = blocking.spawn_blocking(move || {
        blocker.recv().expect("the test releases the lookup");
        lookup_produced.store(true, Ordering::SeqCst);
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Other,
            "the abandoned lookup failed",
        )))
    });
    drop(tasks.spawn(consume_abandoned_lookup(join)));
    assert!(
        !produced.load(Ordering::SeqCst),
        "the abandoned lookup is still waiting"
    );
    release.send(()).expect("the lookup is still waiting");
    blocking.close();
    blocking.wait().await;
    drain(&tasks).await;
    assert!(produced.load(Ordering::SeqCst));
    let text = logs.text();
    assert!(text.contains("an abandoned database lookup failed"));
    assert!(text.contains("the abandoned lookup failed"));
}

#[tokio::test(start_paused = true)]
async fn cancelled_child_not_queried_for_exception() {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let completed = Arc::new(AtomicBool::new(false));
    let lookup_completed = Arc::clone(&completed);
    let lookup = async move {
        sleep(Duration::from_secs(1)).await;
        lookup_completed.store(true, Ordering::SeqCst);
        Ok::<i32, DbLookupError>(7_i32)
    };
    let lookup_request = request("cancelled", Some(0.02_f64), 10.0, &tracker);
    let error = bounded_db_lookup(lookup, lookup_request, &tasks)
        .await
        .expect_err("the deadline wins over the slow lookup");
    assert_deadline(&error, "cancelled", 0.02);
    drain(&tasks).await;
    assert!(!completed.load(Ordering::SeqCst));
    assert!(!matches!(error, DbLookupError::Task(_)));
}

#[tokio::test]
async fn abandoned_panic_logged() {
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    let handle: JoinHandle<Result<(), DbLookupError>> =
        tasks.spawn(async { panic!("the abandoned lookup panicked") });
    consume_abandoned_lookup(handle).await;
    drain(&tasks).await;
    assert!(logs.text().contains("an abandoned database lookup failed"));
}

#[tokio::test]
async fn abandoned_cancellation_silent() {
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    let handle: JoinHandle<Result<(), DbLookupError>> = tasks.spawn(pending());
    handle.abort();
    consume_abandoned_lookup(handle).await;
    drain(&tasks).await;
    assert!(!logs.text().contains("an abandoned database lookup failed"));
}

#[tokio::test]
async fn caller_cancel_no_stall_hit() {
    let tracker = Arc::new(stall_tracker());
    let signal = Arc::new(Notify::new());
    let (started, started_rx) = oneshot::channel();
    let lookup = {
        let guard = NotifyOnDrop(Arc::clone(&signal));
        async move {
            let _guard = guard;
            assert!(started.send(()).is_ok());
            pending::<Result<i32, DbLookupError>>().await
        }
    };
    let caller_tracker = Arc::clone(&tracker);
    let mut callers = JoinSet::new();
    callers.spawn(async move {
        let tasks = TaskTracker::new();
        let lookup_request = LookupRequest {
            name: "slow",
            deadline_seconds: Some(30.0_f64),
            default_deadline_seconds: 10.0,
            tracker: caller_tracker.as_ref(),
        };
        bounded_db_lookup(lookup, lookup_request, &tasks).await
    });
    started_rx
        .await
        .expect("the lookup starts before the caller is cancelled");
    callers.abort_all();
    let joined = callers
        .join_next()
        .await
        .expect("the caller task is tracked")
        .expect_err("the caller was cancelled");
    assert!(joined.is_cancelled());
    timeout(Duration::from_secs(5), signal.notified())
        .await
        .expect("the child lookup is cancelled");
    assert!(!tracker.stalled_within(f64::INFINITY));
}

#[tokio::test]
async fn existing_deadline_exception_unchanged() {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let existing = DbLookupDeadlineExceeded {
        lookup: "inner".to_owned(),
        deadline_seconds: 3.0,
        cause: None,
    };
    let lookup = async move { Err::<i32, DbLookupError>(DbLookupError::Deadline(existing)) };
    let lookup_request = request("outer", Some(7.0_f64), 7.0, &tracker);
    let error = bounded_db_lookup(lookup, lookup_request, &tasks)
        .await
        .expect_err("the inner deadline is rethrown");
    assert_deadline(&error, "inner", 3.0);
    assert_eq!(error.to_string(), "inner lookup did not answer within 3s");
    assert!(!tracker.stalled_within(f64::INFINITY));
}

#[tokio::test]
async fn timeout_cause_preserved() {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let elapsed = timeout(Duration::ZERO, pending::<()>())
        .await
        .expect_err("the zero timer fires before the pending future");
    let lookup = async move { Err::<i32, DbLookupError>(DbLookupError::Elapsed(elapsed)) };
    let lookup_request = request("cache", Some(2.5_f64), 10.0, &tracker);
    let error = bounded_db_lookup(lookup, lookup_request, &tasks)
        .await
        .expect_err("the inner timeout is wrapped");
    let DbLookupError::Deadline(exceeded) = &error else {
        panic!("the lookup must fail with a deadline, got {error:?}");
    };
    assert_eq!(exceeded.lookup, "cache");
    assert_eq!(exceeded.deadline_seconds.to_bits(), 2.5_f64.to_bits());
    assert!(matches!(
        exceeded.cause.as_deref(),
        Some(DbLookupError::Elapsed(_))
    ));
    assert!(tracker.stalled_within(f64::INFINITY));
    assert_eq!(error.to_string(), "cache lookup did not answer within 2.5s");
}

#[tokio::test]
async fn unrelated_query_error_unchanged() {
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let lookup = async {
        Err::<i32, DbLookupError>(DbLookupError::Database(database_error(
            ErrorType::Other,
            "boom",
        )))
    };
    let lookup_request = request("cache", Some(2.5_f64), 10.0, &tracker);
    let error = bounded_db_lookup(lookup, lookup_request, &tasks)
        .await
        .expect_err("the query error passes through");
    let DbLookupError::Database(db_error) = &error else {
        panic!("the query error must pass through, got {error:?}");
    };
    assert_eq!(db_error.error_type, ErrorType::Other);
    assert_eq!(db_error.message, "boom");
    assert_eq!(error.to_string(), "boom");
    assert!(!tracker.stalled_within(f64::INFINITY));
}

async fn postgres_pool() -> (PostgresFixture, Pool) {
    let configured = Environment::parse_from(["database-failures-tests"])
        .url
        .map(SecretString::from);
    let mut provision = PostgresFixture::provision_with_statements(
        configured.as_ref(),
        vec![PROBE_TABLE.to_owned()],
    );
    let fixture = provision
        .resolve()
        .await
        .expect("the fixture provisions a database");
    let config: Config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .expect("the test pool builds");
    (fixture, pool)
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn blocked_gate_lookup(
    semaphore: Arc<Semaphore>,
    pool: Pool,
    acquired: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
) -> Result<(), DbLookupError> {
    let _permit = semaphore.acquire_owned().await.expect("the gate is open");
    acquired.store(true, Ordering::SeqCst);
    let session = pool.get().await.expect("the fixture connects");
    session
        .query("SELECT pg_sleep(5)", &[])
        .await
        .expect("the sleep query answers");
    finished.store(true, Ordering::SeqCst);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_blocked_lookup_deadline() {
    let (fixture, pool) = postgres_pool().await;
    let tasks = TaskTracker::new();
    let tracker = stall_tracker();
    let gate = LoopBoundSemaphore {
        value: 1,
        slot: Mutex::new(None),
    };
    let semaphore = gate.current().expect("a runtime is running");
    let acquired = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let lookup = blocked_gate_lookup(
        Arc::clone(&semaphore),
        pool.clone(),
        Arc::clone(&acquired),
        Arc::clone(&finished),
    );
    let lookup_request = request("user_api_key_cache", Some(0.5_f64), 10.0, &tracker);
    let error = bounded_db_lookup(lookup, lookup_request, &tasks)
        .await
        .expect_err("the blocked query cannot answer inside the deadline");
    assert_deadline(&error, "user_api_key_cache", 0.5);
    assert!(!finished.load(Ordering::SeqCst));
    assert!(tracker.stalled_within(f64::INFINITY));
    drain(&tasks).await;
    assert!(acquired.load(Ordering::SeqCst));
    assert_eq!(semaphore.available_permits(), 1);
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}

async fn committed_write(pool: Pool) -> Result<i32, DbLookupError> {
    let mut session = pool.get().await.expect("the fixture connects");
    let transaction = session.transaction().await.expect("the transaction opens");
    transaction
        .execute(
            r#"INSERT INTO "LiteLLM_DbLookupProbe" (id, note) VALUES ('probe', 'committed')"#,
            &[],
        )
        .await
        .expect("the probe row inserts");
    transaction.commit().await.expect("the probe row commits");
    Ok(1_i32)
}

#[tokio::test]
async fn postgres_metric_failure_keeps_committed_write() {
    let (fixture, pool) = postgres_pool().await;
    let tasks = TaskTracker::new();
    let sink = Arc::new(RecordingSink::default());
    sink.refuse_success.store(true, Ordering::SeqCst);
    let resolver = RecordingResolver::default();
    let call = metric_call("db_write");
    let output = run_metric(
        committed_write(pool.clone()),
        &call,
        &sink,
        &resolver,
        &tasks,
    )
    .await
    .expect("the committed write stays a success");
    assert_eq!(output, 1_i32);
    drain(&tasks).await;
    assert_eq!(successes(&sink).len(), 1);
    let session = pool.get().await.expect("the verification connects");
    let row = session
        .query_one(
            r#"SELECT note FROM "LiteLLM_DbLookupProbe" WHERE id = 'probe'"#,
            &[],
        )
        .await
        .expect("the probe row persists");
    assert_eq!(row.get::<_, String>(0), "committed");
    drop(session);
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}
