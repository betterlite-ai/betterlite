use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::request_log_indexes::database::IndexDatabase as _;
use crate::request_log_indexes::locks::{HANDOVER, with_bounded_lock};

use super::support::{
    API_KEY_INDEX, FakeDatabase, FakeError, FakeFault, FixedJitter, RecordingPause, SPEND_LOGS,
    index_builder, index_by_name, index_dependencies, informed, install_logs, invalid_index,
    occurrences, partitioned_table, plain_table, run_ensure, valid_index, warned,
};

const ATTACH: &str =
    "ALTER INDEX \"public\".\"parent_idx\" ATTACH PARTITION \"public\".\"child_idx\"";

fn fake_with_retry_fault() -> FakeDatabase {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .statement_faults
            .push(("ALTER INDEX".to_owned(), FakeFault::LockNotAvailable));
    });
    fake
}

#[tokio::test]
async fn coordinator_busy_no_retry() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default();
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    let called = AtomicUsize::new(0);
    let outcome = with_bounded_lock(&builder, "creating the parent index idx", || {
        Box::pin(async {
            called.fetch_add(1, Ordering::SeqCst);
            Ok::<bool, FakeError>(true)
        })
    })
    .await
    .unwrap();
    assert!(!outcome);
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert_eq!(fake.state().lock_attempts, 1);
    assert!(pause.delays().is_empty());
    assert_eq!(
        fake.state().statements,
        ["SET lock_timeout = '200ms'", "SET lock_timeout = 0"]
    );
    assert!(informed(
        &logs.contents(),
        "Another process holds the migration lock, leaving the request-log indexes to the next index build"
    ));
}

#[tokio::test]
async fn bounded_lock_ten_attempts_and_jitter() {
    let (logs, _guard) = install_logs();
    let fake = fake_with_retry_fault();
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    let outcome = with_bounded_lock(&builder, "attaching child_idx", || {
        Box::pin(async { fake.execute(ATTACH).await.map(|()| true) })
    })
    .await
    .unwrap();
    assert!(!outcome);
    assert_retry_schedule(&pause, &jitter);
    assert_retry_state(&fake);
    assert_retry_logs(&logs.contents());
}

#[tokio::test]
async fn bounded_lock_jitter_upper_bound() {
    let fake = fake_with_retry_fault();
    let jitter = FixedJitter::new(u64::MAX);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    let outcome = with_bounded_lock(&builder, "attaching child_idx", || {
        Box::pin(async { fake.execute(ATTACH).await.map(|()| true) })
    })
    .await
    .unwrap();
    assert!(!outcome);
    assert_eq!(
        pause.delays(),
        [
            Duration::from_nanos(499_999_999),
            Duration::from_nanos(999_999_999),
            Duration::from_nanos(1_999_999_999),
            Duration::from_nanos(3_999_999_999),
            Duration::from_nanos(7_999_999_999),
            Duration::from_nanos(7_999_999_999),
            Duration::from_nanos(7_999_999_999),
            Duration::from_nanos(7_999_999_999),
            Duration::from_nanos(7_999_999_999),
        ]
    );
    assert_eq!(
        jitter.draws(),
        [
            250_000_000,
            500_000_000,
            1_000_000_000,
            2_000_000_000,
            4_000_000_000,
            4_000_000_000,
            4_000_000_000,
            4_000_000_000,
            4_000_000_000,
        ]
    );
    assert!(pause.locked_during().iter().all(|locked| !locked));
}

fn assert_retry_schedule(pause: &RecordingPause, jitter: &FixedJitter) {
    let expected = [
        Duration::from_millis(250),
        Duration::from_millis(500),
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
        Duration::from_secs(4),
        Duration::from_secs(4),
        Duration::from_secs(4),
        Duration::from_secs(4),
    ];
    assert_eq!(pause.delays(), expected);
    let nanos: Vec<u64> = expected
        .iter()
        .map(|delay| u64::try_from(delay.as_nanos()).unwrap())
        .collect();
    assert_eq!(jitter.draws(), nanos);
    assert!(pause.locked_during().iter().all(|locked| !locked));
}

fn assert_retry_state(fake: &FakeDatabase) {
    assert_eq!(fake.state().lock_attempts, 10);
    assert_eq!(fake.state().release_attempts, 10);
    assert_eq!(fake.state().statements.len(), 30);
}

fn assert_retry_logs(logs: &str) {
    assert_eq!(
        occurrences(
            logs,
            "Waiting for open transactions before attaching child_idx"
        ),
        10
    );
    assert!(warned(
        logs,
        "Could not get the lock for attaching child_idx without holding writes back, leaving it for the next index build"
    ));
}

#[tokio::test]
async fn bounded_lock_reset_on_every_exit() {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .once_faults
            .push(("ALTER INDEX".to_owned(), FakeFault::LockNotAvailable));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    let outcome = with_bounded_lock(&builder, "attaching child_idx", || {
        Box::pin(async { fake.execute(ATTACH).await.map(|()| true) })
    })
    .await
    .unwrap();
    assert!(outcome);
    assert_eq!(pause.delays(), [Duration::from_millis(250)]);
    assert_eq!(
        fake.state().statements,
        [
            "SET lock_timeout = '200ms'",
            ATTACH,
            "SET lock_timeout = 0",
            "SET lock_timeout = '200ms'",
            ATTACH,
            "SET lock_timeout = 0",
        ]
    );
}

#[tokio::test]
async fn lock_reset_error_propagates() {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .statement_faults
            .push(("SET lock_timeout = 0".to_owned(), FakeFault::Database));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    let error = with_bounded_lock(&builder, "attaching child_idx", || {
        Box::pin(async { fake.execute(ATTACH).await.map(|()| true) })
    })
    .await
    .unwrap_err();
    assert_eq!(error.fault, FakeFault::Database);
}

#[tokio::test]
async fn reset_error_is_a_database_failure() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(partitioned_table())));
        state
            .statement_faults
            .push(("SET lock_timeout = 0".to_owned(), FakeFault::Database));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    assert!(
        !run_ensure(&fake, &[index_by_name(API_KEY_INDEX)], &jitter, &pause)
            .await
            .unwrap()
    );
    assert!(warned(
        &logs.contents(),
        "Could not build the request-log indexes, leaving them for the next index build"
    ));
}

#[tokio::test]
async fn non_lock_database_error_not_retried() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(partitioned_table())));
        state
            .statement_faults
            .push(("CREATE INDEX".to_owned(), FakeFault::Database));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    assert!(
        !run_ensure(&fake, &[index_by_name(API_KEY_INDEX)], &jitter, &pause)
            .await
            .unwrap()
    );
    assert_eq!(fake.state().lock_attempts, 1);
    assert!(pause.delays().is_empty());
    assert!(warned(
        &logs.contents(),
        "Could not build the request-log indexes, leaving them for the next index build"
    ));
}

#[tokio::test]
async fn false_step_not_retried() {
    let fake = FakeDatabase::default().working();
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    let outcome = with_bounded_lock(&builder, "attaching child_idx", || {
        Box::pin(async { Ok::<bool, FakeError>(false) })
    })
    .await
    .unwrap();
    assert!(!outcome);
    assert_eq!(fake.state().lock_attempts, 1);
    assert!(pause.delays().is_empty());
    assert_eq!(
        fake.state().statements,
        ["SET lock_timeout = '200ms'", "SET lock_timeout = 0"]
    );
}

#[tokio::test]
async fn sleeps_release_coordinator() {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(plain_table())));
        state
            .index_state_sequence
            .push_back(Ok(Some(invalid_index(SPEND_LOGS))));
        state
            .index_state_sequence
            .push_back(Ok(Some(invalid_index(SPEND_LOGS))));
        state
            .index_state_sequence
            .push_back(Ok(Some(valid_index(SPEND_LOGS))));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    assert!(
        run_ensure(&fake, &[index_by_name(API_KEY_INDEX)], &jitter, &pause)
            .await
            .unwrap()
    );
    assert_eq!(pause.delays(), [HANDOVER]);
    assert_eq!(pause.locked_during(), [false]);
    assert!(!fake.state().locked);
}
