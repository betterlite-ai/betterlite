#![cfg(test)]

use std::time::Duration;

use tokio_util::task::TaskTracker;

use super::{CallbackEvent, ClickHouseSpendLogger};
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
    let logger = ClickHouseSpendLogger { batch };
    (logger, storage, tasks)
}

fn completion() -> CallbackKwargs {
    kwargs(r#"{"standard_logging_object": {"id": "chatcmpl-1", "call_type": "acompletion"}}"#)
}

fn event<'event>(
    kwargs: &'event CallbackKwargs,
    redact: bool,
    lens: &'event LensAnalysis,
) -> CallbackEvent<'event> {
    CallbackEvent {
        kwargs,
        redact,
        lens,
    }
}

#[tokio::test(start_paused = true)]
async fn log_enqueues_a_spend_logs_row() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 3));
    let lens = LensAnalysis::default();
    let completion = completion();
    logger.log(&event(&completion, false, &lens));
    wait_for_inserts(&storage, 1).await;
    let calls = storage.calls();
    assert_eq!(calls[0].table, SPEND_LOGS_TABLE);
    assert_eq!(calls[0].rows.len(), 1);
    assert_eq!(calls[0].rows[0].request_id, "chatcmpl-1");
}

#[tokio::test(start_paused = true)]
async fn log_ignores_trace_ingest_calls() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 3));
    let lens = LensAnalysis::default();
    let ingest =
        kwargs(r#"{"standard_logging_object": {"id": "trace", "call_type": "/v1/traces/span"}}"#);
    logger.log(&event(&ingest, false, &lens));
    logger.batch.flush_queue().await;
    assert_eq!(storage.insert_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn log_ignores_events_without_a_standard_logging_object() {
    let (logger, storage, _tasks) = build(settings(1, LONG_INTERVAL, 3));
    let lens = LensAnalysis::default();
    logger.log(&event(&kwargs("{}"), false, &lens));
    logger.batch.flush_queue().await;
    assert_eq!(storage.insert_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn success_and_failure_events_enqueue_the_same_row() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    let lens = LensAnalysis::default();
    let completion = completion();
    let callback = event(&completion, false, &lens);
    logger.async_log_success_event(&callback).await;
    logger.async_log_failure_event(&callback).await;
    logger.batch.flush_queue().await;
    let calls = storage.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].rows.len(), 2);
    assert_eq!(calls[0].rows[0], calls[0].rows[1]);
}

#[tokio::test(start_paused = true)]
async fn log_redacts_messages_and_response_when_message_logging_is_disabled() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    let lens = LensAnalysis::default();
    let completion = kwargs(
        r#"{"standard_logging_object": {
            "id": "chatcmpl-1",
            "call_type": "acompletion",
            "messages": [{"role": "user", "content": "hi"}],
            "response": {"choices": []}
        }}"#,
    );
    logger.log(&event(&completion, true, &lens));
    logger.batch.flush_queue().await;
    let calls = storage.calls();
    assert_eq!(calls[0].rows[0].messages, "");
    assert_eq!(calls[0].rows[0].response, "");
}

#[tokio::test(start_paused = true)]
async fn a_completion_row_keeps_its_messages_when_message_logging_is_enabled() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    let lens = LensAnalysis::default();
    let completion = kwargs(
        r#"{"standard_logging_object": {
            "id": "chatcmpl-1",
            "call_type": "acompletion",
            "messages": [{"role": "user", "content": "hi"}]
        }}"#,
    );
    logger.log(&event(&completion, false, &lens));
    logger.batch.flush_queue().await;
    assert_eq!(
        storage.calls()[0].rows[0].messages,
        r#"[{"role":"user","content":"hi"}]"#
    );
}

#[tokio::test(start_paused = true)]
async fn a_row_from_another_request_stays_outside_the_lens_scope() {
    let (logger, storage, _tasks) = build(settings(10, LONG_INTERVAL, 3));
    let analysis = LensAnalysis::default();
    let other_request = LensAnalysis::default();
    let completion = completion();
    let scope = analysis.lens_analysis();
    logger.log(&event(&completion, false, &analysis));
    logger.log(&event(&completion, false, &other_request));
    drop(scope);
    logger.batch.flush_queue().await;
    let calls = storage.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].rows[0]
            .metadata
            .contains(r#""litellm_lens_internal":true"#)
    );
    assert!(
        calls[0].rows[1]
            .metadata
            .contains(r#""litellm_lens_internal":false"#)
    );
}
