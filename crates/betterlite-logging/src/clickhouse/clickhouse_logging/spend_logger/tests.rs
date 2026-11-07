#![cfg(test)]

use std::time::Duration;

use tokio_util::task::TaskTracker;

use super::ClickHouseSpendLogger;
use crate::clickhouse::clickhouse_logging::batch_logger::ClickHouseBatchLogger;
use crate::clickhouse::clickhouse_logging::context::LensAnalysis;
use crate::clickhouse::clickhouse_logging::payload::CallbackKwargs;
use crate::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use crate::clickhouse::clickhouse_logging::settings::ClickHouseSettings;
use crate::clickhouse::clickhouse_logging::tests::support::{
    FakeStorage, kwargs, settings, wait_for_inserts,
};

const LONG_INTERVAL: Duration = Duration::from_secs(3600);

fn build(
    configuration: ClickHouseSettings,
) -> (ClickHouseSpendLogger<FakeStorage>, FakeStorage, TaskTracker) {
    let storage = FakeStorage::default();
    let tasks = TaskTracker::new();
    let batch = ClickHouseBatchLogger::new(
        storage.clone(),
        SPEND_LOGS_TABLE,
        configuration,
        tasks.clone(),
    );
    let lens = LensAnalysis::default();
    (ClickHouseSpendLogger { batch, lens }, storage, tasks)
}

fn completion() -> CallbackKwargs {
    kwargs(r#"{"standard_logging_object": {"id": "chatcmpl-1", "call_type": "acompletion"}}"#)
}

#[tokio::test(start_paused = true)]
async fn log_enqueues_a_spend_logs_row() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 3));
    logger.log(&completion(), false);
    wait_for_inserts(&storage, 1).await;
    let calls = storage.calls();
    assert_eq!(calls[0].table, SPEND_LOGS_TABLE);
    assert_eq!(calls[0].rows.len(), 1);
    assert_eq!(calls[0].rows[0].request_id, "chatcmpl-1");
}

#[tokio::test(start_paused = true)]
async fn log_ignores_trace_ingest_calls() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 3));
    let ingest =
        kwargs(r#"{"standard_logging_object": {"id": "trace", "call_type": "/v1/traces/span"}}"#);
    logger.log(&ingest, false);
    logger.batch.flush_queue().await;
    assert_eq!(storage.insert_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn log_ignores_events_without_a_standard_logging_object() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 3));
    logger.log(&kwargs("{}"), false);
    logger.batch.flush_queue().await;
    assert_eq!(storage.insert_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn success_and_failure_events_enqueue_the_same_row() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    let event = completion();
    logger.async_log_success_event(&event, false).await;
    logger.async_log_failure_event(&event, false).await;
    logger.batch.flush_queue().await;
    let calls = storage.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].rows.len(), 2);
    assert_eq!(calls[0].rows[0], calls[0].rows[1]);
}

#[tokio::test(start_paused = true)]
async fn log_redacts_messages_and_response_when_message_logging_is_disabled() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    let event = kwargs(
        r#"{"standard_logging_object": {
            "id": "chatcmpl-1",
            "call_type": "acompletion",
            "messages": [{"role": "user", "content": "hi"}],
            "response": {"choices": []}
        }}"#,
    );
    logger.log(&event, true);
    logger.batch.flush_queue().await;
    let calls = storage.calls();
    assert_eq!(calls[0].rows[0].messages, "");
    assert_eq!(calls[0].rows[0].response, "");
}

#[tokio::test(start_paused = true)]
async fn a_completion_row_keeps_its_messages_when_message_logging_is_enabled() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    let event = kwargs(
        r#"{"standard_logging_object": {
            "id": "chatcmpl-1",
            "call_type": "acompletion",
            "messages": [{"role": "user", "content": "hi"}]
        }}"#,
    );
    logger.log(&event, false);
    logger.batch.flush_queue().await;
    assert_eq!(
        storage.calls()[0].rows[0].messages,
        r#"[{"role":"user","content":"hi"}]"#
    );
}
