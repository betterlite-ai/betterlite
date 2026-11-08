#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::ExposeSecret as _;
use tokio::task::yield_now;
use tokio::time::advance;
use tokio_util::task::TaskTracker;
use tracing::subscriber::set_default;

use super::ClickHouseBatchLogger;
use crate::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use crate::clickhouse::clickhouse_logging::settings::{
    ClickHouseSettings, DATABASE_VARIABLE, DEFAULT_BATCH_SIZE, URL_VARIABLE,
};
use crate::clickhouse::clickhouse_logging::tests::support::{
    Captured, FakeStorage, row, settings, wait_for_inserts,
};

const LONG_INTERVAL: Duration = Duration::from_secs(3600);

fn build(
    configuration: ClickHouseSettings,
) -> (ClickHouseBatchLogger<FakeStorage>, FakeStorage, TaskTracker) {
    let storage = FakeStorage::default();
    let tasks = TaskTracker::new();
    let logger = ClickHouseBatchLogger::new(
        storage.clone(),
        SPEND_LOGS_TABLE,
        configuration,
        tasks.clone(),
    );
    (logger, storage, tasks)
}

fn queue(logger: &ClickHouseBatchLogger<FakeStorage>, request_ids: &[&str]) {
    for request_id in request_ids {
        let _enqueued = logger.inner.batch.enqueue(row(request_id));
    }
}

async fn settle() {
    for _ in 0_u8..8 {
        yield_now().await;
    }
}

#[tokio::test]
async fn from_environment_builds_the_storage_from_the_default_database_and_url() {
    let seen = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);
    let tasks = TaskTracker::new();
    let logger = ClickHouseBatchLogger::from_environment(
        &BTreeMap::new(),
        SPEND_LOGS_TABLE,
        tasks,
        move |configuration: &ClickHouseSettings| {
            *sink.lock().unwrap_or_else(PoisonError::into_inner) = Some((
                configuration.database.clone(),
                configuration.url.expose_secret().to_owned(),
            ));
            FakeStorage::default()
        },
    )
    .expect("the logger builds");
    drop(logger);
    let observed = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(observed, Some(("litellm".to_owned(), String::new())));
}

#[tokio::test]
async fn from_environment_passes_the_configured_database_and_url() {
    let environment = BTreeMap::from([
        (DATABASE_VARIABLE.to_owned(), "traces".to_owned()),
        (URL_VARIABLE.to_owned(), "http://clickhouse:8123".to_owned()),
    ]);
    let seen = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);
    let tasks = TaskTracker::new();
    let logger = ClickHouseBatchLogger::from_environment(
        &environment,
        SPEND_LOGS_TABLE,
        tasks,
        move |configuration: &ClickHouseSettings| {
            *sink.lock().unwrap_or_else(PoisonError::into_inner) = Some((
                configuration.database.clone(),
                configuration.url.expose_secret().to_owned(),
                configuration.batch_size,
            ));
            FakeStorage::default()
        },
    )
    .expect("the logger builds");
    drop(logger);
    let observed = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        observed,
        Some((
            "traces".to_owned(),
            "http://clickhouse:8123".to_owned(),
            DEFAULT_BATCH_SIZE
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn enqueue_flushes_as_soon_as_a_full_batch_is_queued() {
    let (logger, storage, _tasks) = build(settings(2, LONG_INTERVAL, 3));
    logger.enqueue([row("a"), row("b")]);
    wait_for_inserts(&storage, 1).await;
    assert_eq!(
        storage.sent_request_ids(),
        vec![vec!["a".to_owned(), "b".to_owned()]]
    );
    assert_eq!(storage.calls()[0].table, SPEND_LOGS_TABLE);
}

#[tokio::test(start_paused = true)]
async fn enqueue_waits_for_the_flush_interval_before_sending_a_partial_batch() {
    let (logger, storage, _tasks) = build(settings(2, Duration::from_secs(5), 3));
    logger.enqueue([row("a")]);
    settle().await;
    assert_eq!(storage.insert_count(), 0);
    advance(Duration::from_secs(5)).await;
    wait_for_inserts(&storage, 1).await;
    assert_eq!(storage.sent_request_ids(), vec![vec!["a".to_owned()]]);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_logger_stops_the_periodic_flush_without_sending() {
    let (logger, storage, tasks) = build(settings(2, Duration::from_secs(5), 3));
    logger.enqueue([row("a")]);
    drop(logger);
    advance(Duration::from_secs(5)).await;
    tasks.close();
    tasks.wait().await;
    assert_eq!(storage.insert_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn flush_queue_sends_batches_of_the_configured_size() {
    let (logger, storage, _tasks) = build(settings(2, LONG_INTERVAL, 3));
    queue(&logger, &["a", "b", "c", "d", "e"]);
    logger.flush_queue().await;
    assert_eq!(
        storage.sent_request_ids(),
        vec![
            vec!["a".to_owned(), "b".to_owned()],
            vec!["c".to_owned(), "d".to_owned()],
            vec!["e".to_owned()],
        ]
    );
    assert_eq!(logger.stats().rows_written, 5);
}

#[tokio::test(start_paused = true)]
async fn flush_queue_keeps_rows_enqueued_during_the_insert() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    queue(&logger, &["a", "b"]);
    storage.arrive_during_insert(logger.inner.batch.clone(), row("c"));
    logger.flush_queue().await;
    assert_eq!(
        storage.sent_request_ids(),
        vec![vec!["a".to_owned(), "b".to_owned()]]
    );
    logger.flush_queue().await;
    assert_eq!(
        storage.sent_request_ids(),
        vec![vec!["a".to_owned(), "b".to_owned()], vec!["c".to_owned()]]
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_insert_requeues_the_batch_at_the_front() {
    let (logger, storage, _tasks) = build(settings(2, LONG_INTERVAL, 3));
    queue(&logger, &["a", "b", "c"]);
    storage.fail_next(1);
    logger.flush_queue().await;
    assert_eq!(
        storage.sent_request_ids(),
        vec![vec!["a".to_owned(), "b".to_owned()]]
    );
    assert_eq!(logger.stats().rows_written, 0);
    logger.flush_queue().await;
    assert_eq!(
        storage.sent_request_ids(),
        vec![
            vec!["a".to_owned(), "b".to_owned()],
            vec!["a".to_owned(), "b".to_owned()],
            vec!["c".to_owned()],
        ]
    );
    assert_eq!(logger.stats().rows_written, 3);
}

#[tokio::test(start_paused = true)]
async fn a_failed_insert_is_dropped_after_the_maximum_attempts() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 2));
    queue(&logger, &["a"]);
    storage.fail_next(2);
    logger.flush_queue().await;
    logger.flush_queue().await;
    assert_eq!(logger.stats().rows_dropped, 1);
    assert_eq!(logger.inner.buffer_depth(), 0);
    logger.flush_queue().await;
    assert_eq!(storage.insert_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_batch_is_logged_as_an_error() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = set_default(subscriber);
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 1));
    queue(&logger, &["a"]);
    storage.fail_next(1);
    logger.flush_queue().await;
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).expect("utf8");
    assert!(output.contains("ERROR"), "{output}");
    assert!(output.contains("dropped rows"), "{output}");
}

#[tokio::test(start_paused = true)]
async fn a_successful_insert_resets_the_failure_count() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 3));
    queue(&logger, &["a"]);
    storage.fail_next(1);
    logger.flush_queue().await;
    assert_eq!(logger.inner.buffer_depth(), 1);
    logger.flush_queue().await;
    assert_eq!(logger.inner.buffer_depth(), 0);
    storage.fail_next(1);
    queue(&logger, &["b"]);
    logger.flush_queue().await;
    assert_eq!(logger.stats().rows_dropped, 0);
    assert_eq!(logger.inner.buffer_depth(), 1);
}

#[tokio::test(start_paused = true)]
async fn is_full_reports_the_buffered_row_cap_without_dropping_rows() {
    let mut configuration = settings(10, LONG_INTERVAL, 3);
    configuration.max_buffered_rows = 2;
    let (logger, _storage, _tasks) = build(configuration);
    queue(&logger, &["a"]);
    assert!(!logger.is_full());
    queue(&logger, &["b", "c"]);
    assert!(logger.is_full());
    assert_eq!(logger.inner.buffer_depth(), 3);
}

#[tokio::test(start_paused = true)]
async fn shutdown_flushes_the_remaining_rows() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    queue(&logger, &["a", "b"]);
    logger.shutdown().await;
    assert_eq!(
        storage.sent_request_ids(),
        vec![vec!["a".to_owned(), "b".to_owned()]]
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_drops_rows_after_the_retry_limit() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 2));
    queue(&logger, &["a"]);
    storage.fail_next(2);
    logger.shutdown().await;
    assert_eq!(logger.stats().rows_dropped, 1);
    assert_eq!(logger.inner.buffer_depth(), 0);
}
