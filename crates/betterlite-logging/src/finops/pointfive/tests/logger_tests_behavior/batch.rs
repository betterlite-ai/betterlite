use tokio::task::JoinHandle;

use crate::callback_logger::CallbackLogger as _;
use crate::finops::pointfive::logger::PointFiveLogger;

use super::support::{
    FakeHttp, PRESIGNED, Reply, batching, client, event, logger_started, payload,
    take_periodic_task, target_body, wait_until,
};

fn upload_pair(index: usize) -> [Reply; 2] {
    [
        Reply::with_body(
            200,
            &target_body(&format!("{PRESIGNED}/{index}"), &format!("objects/{index}")),
        ),
        Reply::answer(200),
    ]
}

async fn queued(logger: &PointFiveLogger, id: &str) {
    logger
        .async_log_success_event(event(payload(id)))
        .await
        .expect("the event is queued");
}

#[tokio::test]
async fn the_threshold_starts_a_background_flush() {
    let mut replies = vec![Reply::answer(200)];
    replies.extend(upload_pair(0));
    let http = FakeHttp::scripted(replies);
    let logger = logger_started(batching(2), client(&http, 3));
    wait_until(|| !http.pings().is_empty()).await;
    queued(&logger, "evt-1").await;
    queued(&logger, "evt-2").await;
    wait_until(|| !http.uploads().is_empty()).await;
    wait_until(|| logger.core.queue.is_empty()).await;
    assert_eq!(http.uploads().len(), 1);
    logger.shutdown.cancel();
    take_periodic_task(&logger)
        .await
        .expect("the periodic flush ends");
}

#[tokio::test]
async fn an_event_under_the_threshold_starts_nothing() {
    let http = FakeHttp::scripted(vec![Reply::answer(200)]);
    let logger = logger_started(batching(2), client(&http, 1));
    wait_until(|| !http.pings().is_empty()).await;
    queued(&logger, "evt-1").await;
    assert!(
        logger
            .batch_task
            .lock()
            .expect("the lock is not poisoned")
            .is_none()
    );
    logger.shutdown.cancel();
    take_periodic_task(&logger)
        .await
        .expect("the periodic flush ends");
}

#[tokio::test]
async fn a_running_batch_flush_is_not_duplicated() {
    let mut replies = vec![Reply::answer(200)];
    replies.extend(upload_pair(0));
    let http = FakeHttp::scripted(replies);
    let logger = logger_started(batching(2), client(&http, 3));
    wait_until(|| !http.pings().is_empty()).await;
    queued(&logger, "evt-1").await;
    queued(&logger, "evt-2").await;
    logger.start_batch_flush_task();
    assert_eq!(logger.tasks.len(), 2);
    wait_until(|| !http.uploads().is_empty()).await;
    wait_until(|| logger.core.queue.is_empty()).await;
    assert_eq!(http.uploads().len(), 1);
    assert_eq!(http.requests().len(), 3);
    logger.shutdown.cancel();
    take_periodic_task(&logger)
        .await
        .expect("the periodic flush ends");
}

#[tokio::test]
async fn a_finished_batch_flush_is_replaced() {
    let mut replies = vec![Reply::answer(200)];
    replies.extend(upload_pair(0));
    replies.extend(upload_pair(1));
    let http = FakeHttp::scripted(replies);
    let logger = logger_started(batching(2), client(&http, 3));
    wait_until(|| !http.pings().is_empty()).await;
    queued(&logger, "evt-1").await;
    queued(&logger, "evt-2").await;
    wait_until(|| !http.uploads().is_empty()).await;
    wait_until(|| logger.core.queue.is_empty()).await;
    wait_until(|| {
        logger
            .batch_task
            .lock()
            .expect("the lock is not poisoned")
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
    })
    .await;
    queued(&logger, "evt-3").await;
    queued(&logger, "evt-4").await;
    wait_until(|| http.uploads().len() == 2).await;
    wait_until(|| logger.core.queue.is_empty()).await;
    logger.shutdown.cancel();
    take_periodic_task(&logger)
        .await
        .expect("the periodic flush ends");
}
