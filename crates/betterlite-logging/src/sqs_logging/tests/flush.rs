use std::sync::Arc;
use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;

use crate::callback_logger::CallbackLogger as _;
use crate::sqs_logging::SqsLogger;
use crate::sqs_logging::flush::spawn_flush_loop;

use super::support::{Harness, RecordingDelivery, Records, event, payload};

fn body_has_id(body: &str, id: &str) -> bool {
    let decoded = body
        .strip_prefix("Action=SendMessage&Version=2012-11-05&MessageBody=")
        .expect("the form body has the send-message prefix");
    let json = percent_encoding::percent_decode_str(decoded)
        .decode_utf8()
        .expect("the message body is UTF-8");
    json.contains(id)
}

async fn enqueue(logger: &Arc<SqsLogger<RecordingDelivery>>, id: &str) {
    let mut queued = payload();
    queued.id = id.to_owned();
    logger
        .async_log_success_event(event(queued))
        .await
        .expect("the event is recorded");
}

#[tokio::test]
async fn async_send_batch_launches_one_send_per_payload() {
    let (logger, records) = Harness::standard().build();
    enqueue(&logger, "first").await;
    enqueue(&logger, "second").await;
    assert_eq!(logger.async_send_batch(), 2);
    records.wait_for(2).await;
    assert_eq!(records.count(), 2);
    assert_eq!(logger.queue.pending().len(), 2);
}

#[tokio::test]
async fn async_send_batch_sends_nothing_when_empty() {
    let (logger, records) = Harness::standard().build();
    assert_eq!(logger.async_send_batch(), 0);
    yield_now().await;
    assert_eq!(records.count(), 0);
}

#[tokio::test]
async fn flush_queue_sends_and_clears_the_queue() {
    let (logger, records) = Harness::standard().build();
    enqueue(&logger, "only").await;
    logger.flush_queue().await;
    records.wait_for(1).await;
    assert!(logger.queue.pending().is_empty());
}

#[tokio::test]
async fn flush_queue_keeps_an_arrival_for_the_next_flush() {
    let (logger, records) = Harness::standard().build();
    enqueue(&logger, "first-payload").await;
    logger.flush_queue().await;
    enqueue(&logger, "second-payload").await;
    assert_eq!(logger.queue.pending().len(), 1);
    logger.flush_queue().await;
    records.wait_for(2).await;
    assert!(body_has_id(&records.body(0), "first-payload"));
    assert!(body_has_id(&records.body(1), "second-payload"));
    assert!(logger.queue.pending().is_empty());
}

#[tokio::test]
async fn flush_queue_does_nothing_when_empty() {
    let (logger, records) = Harness::standard().build();
    logger.flush_queue().await;
    yield_now().await;
    assert_eq!(records.count(), 0);
}

#[tokio::test]
async fn flush_queue_sends_one_message_per_payload() {
    let (logger, records) = Harness::standard().build();
    enqueue(&logger, "payload-0").await;
    enqueue(&logger, "payload-1").await;
    enqueue(&logger, "payload-2").await;
    logger.flush_queue().await;
    records.wait_for(3).await;
    assert_one_message(&records, 0);
    assert_one_message(&records, 1);
    assert_one_message(&records, 2);
}

fn assert_one_message(records: &Records, index: usize) {
    let body = records.body(index);
    assert!(body.contains("Action=SendMessage"));
    assert!(!body.contains("SendMessageBatch"));
}

#[tokio::test(start_paused = true)]
async fn flush_loop_sends_after_the_interval() {
    let mut harness = Harness::standard();
    harness.settings.batch.flush_interval = Duration::from_secs(10);
    let (logger, records) = harness.build();
    let shutdown = CancellationToken::new();
    spawn_flush_loop(&logger, shutdown.clone());
    yield_now().await;
    enqueue(&logger, "interval").await;
    advance(Duration::from_secs(10)).await;
    yield_now().await;
    let awaited = timeout(Duration::from_secs(1), records.wait_for(1)).await;
    awaited.unwrap();
    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn flush_loop_stops_on_shutdown() {
    let (logger, _records) = Harness::standard().build();
    let shutdown = CancellationToken::new();
    spawn_flush_loop(&logger, shutdown.clone());
    yield_now().await;
    shutdown.cancel();
    logger.tasks.close();
    let awaited = timeout(Duration::from_secs(1), logger.tasks.wait()).await;
    awaited.unwrap();
}
