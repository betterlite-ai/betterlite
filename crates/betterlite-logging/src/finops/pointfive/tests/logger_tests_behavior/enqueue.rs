use betterlite_config::sections::pointfive_params::PointFiveInitParams;

use crate::callback_logger::CallbackLogger as _;

use super::support::{FakeHttp, batching, client, event, logger, payload};

#[tokio::test]
async fn a_success_event_is_buffered() {
    let http = FakeHttp::default();
    let logger = logger(PointFiveInitParams::default(), client(&http, 1));
    logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("a buffered event never fails the request");
    let queued = logger.core.queue.snapshot();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id, "evt-1");
}

#[tokio::test]
async fn a_failure_event_is_buffered() {
    let http = FakeHttp::default();
    let logger = logger(PointFiveInitParams::default(), client(&http, 1));
    logger
        .async_log_failure_event(event(payload("evt-2")))
        .await
        .expect("a buffered event never fails the request");
    let queued = logger.core.queue.snapshot();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id, "evt-2");
}

#[tokio::test]
async fn both_hooks_append_in_arrival_order() {
    let http = FakeHttp::default();
    let logger = logger(PointFiveInitParams::default(), client(&http, 1));
    logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("the success event is queued");
    logger
        .async_log_failure_event(event(payload("evt-2")))
        .await
        .expect("the failure event is queued");
    let ids: Vec<String> = logger
        .core
        .queue
        .snapshot()
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(ids, vec!["evt-1".to_owned(), "evt-2".to_owned()]);
}

#[tokio::test]
async fn events_below_the_batch_size_are_not_uploaded_yet() {
    let http = FakeHttp::default();
    let logger = logger(batching(3), client(&http, 1));
    logger
        .async_log_success_event(event(payload("evt-1")))
        .await
        .expect("the event is queued");
    logger
        .async_log_success_event(event(payload("evt-2")))
        .await
        .expect("the event is queued");
    assert_eq!(logger.core.queue.len(), 2);
    assert!(http.requests().is_empty());
}
