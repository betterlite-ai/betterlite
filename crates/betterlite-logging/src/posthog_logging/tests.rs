#![cfg(test)]

mod async_batch;
mod async_init;
mod async_queue;
mod credentials;
mod distinct_id;
mod event_type;
mod exit;
mod init;
mod metadata;
mod missing_key;
mod properties;
mod support;
mod sync;
mod transport;

use std::sync::Arc;

use crate::callback_logger::CallbackLogger as _;
use crate::posthog_logging::tests::support::{
    ScriptedTransport, default_settings, event, failed_payload, fixed_id, logger_with, payload,
    settle, text,
};

#[tokio::test]
async fn callback_logger_answers_to_the_posthog_name() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, _tasks) = logger_with(default_settings(), &transport, fixed_id);
    assert_eq!(logger.callback_name(), "posthog");
}

#[tokio::test]
async fn callback_logger_queues_async_success_and_failure_events() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, _tasks) = logger_with(default_settings(), &transport, fixed_id);
    logger
        .async_log_success_event(Arc::new(event(payload())))
        .await
        .expect("the success event is queued");
    logger
        .async_log_failure_event(Arc::new(event(failed_payload())))
        .await
        .expect("the failure event is queued");
    assert_eq!(logger.sink.queued_depth(), 2);
}

#[tokio::test]
async fn callback_logger_swallows_an_unbuildable_event() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let mut broken = payload();
    broken.metadata.requester_metadata = Some(text("not an object"));
    logger
        .log_success_event(&event(broken))
        .expect("the sync error is swallowed");
    settle(&tasks).await;
    assert!(transport.requests().is_empty());
    assert_eq!(logger.sink.queued_depth(), 0);
}

#[tokio::test]
async fn callback_logger_carries_the_batch_size_and_mock_mode() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    assert_eq!(logger.settings.batch.batch_size, 100);
    assert!(!logger.settings.mock_mode);
    assert_eq!(logger.sink.queued_depth(), 0);
    logger.stopped.cancel();
    settle(&tasks).await;
}
