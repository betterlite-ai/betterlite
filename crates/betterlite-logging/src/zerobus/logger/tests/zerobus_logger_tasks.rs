use std::sync::Arc;
use std::sync::PoisonError;
use std::time::Duration;

use tokio::task::{Id, JoinHandle};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::callback_logger::CallbackLogger as _;
use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{
    FakeIngest, as_row_ingest, deps, event, expected_ids, logger, params, queued_ids, settle,
    task_alive, until,
};

fn periodic_id(logger: &ZerobusLogger) -> Option<Id> {
    let task = logger
        .periodic
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    task.as_ref().map(JoinHandle::id)
}

fn batch_id(logger: &ZerobusLogger) -> Option<Id> {
    let task = logger.batch.lock().unwrap_or_else(PoisonError::into_inner);
    task.as_ref().map(JoinHandle::id)
}

#[tokio::test]
async fn zerobus_logger_tasks_starts_the_periodic_flush() {
    let tracker = TaskTracker::new();
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut deps = deps(params(100, 3600, false), Some(client));
    deps.tasks = tracker.clone();
    deps.start_periodic_flush = true;
    let logger = ZerobusLogger::new(deps).expect("the logger starts its periodic flush");
    assert_eq!(tracker.len(), 1);
    assert!(task_alive(&logger.periodic));
    logger.shutdown.cancel();
    assert!(
        until(|| !task_alive(&logger.periodic)).await,
        "the periodic task stops on shutdown"
    );
}

#[tokio::test]
async fn zerobus_logger_tasks_stops_the_periodic_flush_when_dropped() {
    let tracker = TaskTracker::new();
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut deps = deps(params(100, 3600, false), Some(client));
    deps.tasks = tracker.clone();
    deps.start_periodic_flush = true;
    let logger = ZerobusLogger::new(deps).expect("the logger starts its periodic flush");
    assert_eq!(tracker.len(), 1);
    drop(logger);
    tracker.close();
    timeout(Duration::from_secs(1), tracker.wait())
        .await
        .expect("dropping the logger stops its periodic flush");
}

#[test]
fn zerobus_logger_tasks_skips_the_periodic_flush_without_a_runtime() {
    let tracker = TaskTracker::new();
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut deps = deps(params(100, 3600, false), Some(client));
    deps.tasks = tracker.clone();
    deps.start_periodic_flush = true;
    let logger = ZerobusLogger::new(deps).expect("the logger builds");
    assert!(tracker.is_empty());
    assert!(!task_alive(&logger.periodic));
}

#[tokio::test]
async fn zerobus_logger_tasks_recreates_a_finished_periodic_task() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut deps = deps(params(100, 3600, false), Some(client));
    deps.start_periodic_flush = true;
    let logger = ZerobusLogger::new(deps).expect("the logger builds");
    let first = periodic_id(&logger).expect("the periodic task exists");
    logger.shutdown.cancel();
    assert!(
        until(|| !task_alive(&logger.periodic)).await,
        "the periodic task stops"
    );
    logger
        .async_log_success_event(event("chatcmpl-r"))
        .await
        .expect("the event is accepted");
    let second = periodic_id(&logger).expect("enqueue recreates the periodic task");
    assert_ne!(first, second);
}

#[tokio::test]
async fn zerobus_logger_tasks_keeps_the_running_periodic_task() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let mut deps = deps(params(100, 3600, false), Some(client));
    deps.start_periodic_flush = true;
    let logger = ZerobusLogger::new(deps).expect("the logger builds");
    let first = periodic_id(&logger).expect("the periodic task exists");
    logger
        .async_log_success_event(event("chatcmpl-k"))
        .await
        .expect("the event is accepted");
    let second = periodic_id(&logger).expect("the periodic task stays");
    assert_eq!(first, second);
}

#[tokio::test]
async fn zerobus_logger_tasks_starts_the_batch_flush_at_the_batch_size() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(2, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-a"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("chatcmpl-b"))
        .await
        .expect("the event is accepted");
    settle(&logger).await;
    assert_eq!(ingest.inserted(), 1);
    assert_eq!(ingest.ids(), expected_ids(&["chatcmpl-a", "chatcmpl-b"]));
}

#[tokio::test]
async fn zerobus_logger_tasks_keeps_one_batch_flush_running() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(1, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-a"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    logger
        .async_log_success_event(event("chatcmpl-b"))
        .await
        .expect("the event is accepted");
    assert_eq!(queued_ids(&logger), expected_ids(&["chatcmpl-b"]));
    assert_eq!(ingest.inserted(), 1);
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    assert_eq!(ingest.inserted(), 1);
}

#[tokio::test]
async fn zerobus_logger_tasks_keeps_the_running_batch_task() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(1, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-a"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    let first = batch_id(&logger).expect("the batch task exists");
    logger
        .async_log_success_event(event("chatcmpl-b"))
        .await
        .expect("the event is accepted");
    let second = batch_id(&logger).expect("the batch task stays");
    assert_eq!(first, second);
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
}

#[test]
fn zerobus_logger_tasks_skips_the_batch_flush_without_a_runtime() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(1, 3600, false), client);
    logger.enqueue(&event("chatcmpl-n"));
    assert_eq!(queued_ids(&logger).len(), 1);
    assert!(!task_alive(&logger.batch));
}
