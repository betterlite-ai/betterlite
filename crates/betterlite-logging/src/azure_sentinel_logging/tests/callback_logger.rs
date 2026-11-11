use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;
use tokio_util::task::TaskTracker;

use super::support::{
    MODEL_URL, Reply, TOKEN_URL, event, model_payload, pending_ids, rig, timed_rig,
};
use crate::batch::logger::BatchSettings;
use crate::callback_logger::CallbackLogger;

#[tokio::test]
async fn callback_logger_is_named_azure_sentinel() {
    let rig = rig(Vec::new(), 10, 50_000);
    assert_eq!(rig.logger.callback_name(), "azure_sentinel");
}

#[tokio::test]
async fn callback_logger_reaches_the_async_events() {
    let rig = rig(Vec::new(), 10, 50_000);
    let concrete_callback = Arc::clone(&rig.logger);
    let callback: Arc<dyn CallbackLogger> = concrete_callback;
    callback
        .async_log_success_event(event(model_payload("one", false)))
        .await
        .expect("the success event is accepted");
    callback
        .async_log_failure_event(event(model_payload("two", true)))
        .await
        .expect("the failure event is accepted");
    assert_eq!(pending_ids(&rig.logger.model.queue), ["one", "two"]);
}

#[tokio::test]
async fn callback_logger_events_send_at_the_batch_size() {
    let rig = rig(vec![Reply::token(), Reply::status(204)], 2, 50_000);
    rig.logger
        .async_log_success_event(event(model_payload("one", false)))
        .await
        .expect("the success event is accepted");
    rig.logger
        .async_log_failure_event(event(model_payload("two", true)))
        .await
        .expect("the failure event is accepted");
    assert_eq!(rig.transport.urls(), [TOKEN_URL, MODEL_URL]);
    assert_eq!(rig.transport.batch_ids(), [["one", "two"]]);
}

#[tokio::test(start_paused = true)]
async fn periodic_flush_sends_on_the_interval_until_stopped() {
    let tracker = TaskTracker::new();
    let rig = timed_rig(
        vec![Reply::token(), Reply::status(204)],
        BatchSettings {
            batch_size: 10,
            flush_interval: Duration::from_secs(5),
            max_queue_size: 50_000,
        },
        Some(tracker.clone()),
    );
    rig.logger
        .async_log_model_event(Some(&model_payload("e1", false)))
        .await;
    sleep(Duration::from_secs(4)).await;
    assert!(rig.transport.calls().is_empty());
    sleep(Duration::from_secs(2)).await;
    assert_eq!(rig.transport.batch_ids(), [["e1"]]);
    rig.stopped.cancel();
    tracker.close();
    tracker.wait().await;
}
