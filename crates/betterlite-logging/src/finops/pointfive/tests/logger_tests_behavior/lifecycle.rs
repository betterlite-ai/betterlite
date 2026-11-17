use crate::callback_logger::CallbackLogger as _;

use super::support::{FakeHttp, batching, client, event, logger, payload, wait_until};

#[tokio::test]
async fn a_dead_periodic_task_is_restarted_by_the_next_event() {
    let http = FakeHttp::default();
    let logger = logger(batching(100), client(&http, 1));
    logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("the event is queued");
    assert!(logger.flush_task_is_alive());
    logger
        .periodic_task
        .lock()
        .expect("the lock is not poisoned")
        .as_ref()
        .expect("the periodic task was started")
        .abort();
    wait_until(|| !logger.flush_task_is_alive()).await;
    logger
        .async_log_success_event(event(payload("evt-2")))
        .await
        .expect("the event is queued");
    assert!(logger.flush_task_is_alive());
    logger
        .periodic_task
        .lock()
        .expect("the lock is not poisoned")
        .as_ref()
        .expect("the periodic task restarted")
        .abort();
    logger.shutdown.cancel();
}

#[tokio::test]
async fn an_alive_periodic_task_is_not_replaced() {
    let http = FakeHttp::default();
    let logger = logger(batching(100), client(&http, 1));
    logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("the event is queued");
    assert_eq!(logger.tasks.len(), 1);
    logger
        .async_log_success_event(event(payload("evt-2")))
        .await
        .expect("the event is queued");
    assert_eq!(logger.tasks.len(), 1);
    logger.shutdown.cancel();
}

#[tokio::test]
async fn the_batch_flush_starts_only_at_the_batch_size() {
    let http = FakeHttp::default();
    let logger = logger(batching(2), client(&http, 1));
    logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("the event is queued");
    assert!(
        logger
            .batch_task
            .lock()
            .expect("the lock is not poisoned")
            .is_none()
    );
    logger
        .async_log_success_event(event(payload("evt-2")))
        .await
        .expect("the event is queued");
    assert!(
        logger
            .batch_task
            .lock()
            .expect("the lock is not poisoned")
            .is_some()
    );
    logger.shutdown.cancel();
}
