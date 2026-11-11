use super::support::{Fixture, Reply, event, payload, record_at};
use crate::callback_logger::CallbackLogger as _;
use crate::newrelic_metrics::queue::{push_record, queue_length};

#[tokio::test]
async fn post_stop_callback_drains_every_success_event() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(202)]);
    let logger = fixture.build();
    assert!(logger.stop());
    fixture.wait_for_tasks().await;
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("a logged success event");
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn post_stop_callback_drains_every_failure_event() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(202)]);
    let logger = fixture.build();
    assert!(logger.stop());
    fixture.wait_for_tasks().await;
    logger
        .async_log_failure_event(event(payload()))
        .await
        .expect("a logged failure event");
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn post_stop_callback_leaves_a_running_logger_below_the_batch_size() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("a logged success event");
    assert!(fixture.transport.requests().is_empty());
    assert_eq!(queue_length(&logger), 1);
}

#[tokio::test]
async fn post_stop_callback_flushes_a_running_logger_at_the_batch_size() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    (0_usize..198).for_each(|index| push_record(&logger, record_at(index)));
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("a logged success event");
    assert!(fixture.transport.requests().is_empty());
    logger
        .async_log_failure_event(event(payload()))
        .await
        .expect("a logged failure event");
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(queue_length(&logger), 0);
}
