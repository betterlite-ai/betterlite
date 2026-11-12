use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::{advance, timeout as wall_timeout};

use crate::callback_logger::CallbackLogger as _;
use crate::posthog_logging::tests::support::{
    ScriptedTransport, default_settings, event, failed_payload, fixed_id, logger_with, payload,
    queued, stop,
};

#[tokio::test]
async fn posthog_async_init_starts_on_the_first_event_only() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    assert!(!logger.async_initialized.load(Ordering::SeqCst));
    assert_eq!(tasks.len(), 0);
    logger
        .async_log_success_event(Arc::new(event(payload())))
        .await
        .expect("the event is queued");
    assert!(logger.async_initialized.load(Ordering::SeqCst));
    assert_eq!(tasks.len(), 1);
    logger
        .async_log_success_event(Arc::new(event(payload())))
        .await
        .expect("the event is queued");
    assert_eq!(tasks.len(), 1);
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_init_flush_before_the_first_event_does_nothing() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let _enqueued = logger.sink.queue.enqueue(queued(&payload()));
    logger.flush_queue().await;
    assert!(transport.requests().is_empty());
    assert_eq!(logger.sink.queued_depth(), 1);
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_init_runs_for_a_failure_event_too() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    logger
        .async_log_failure_event(Arc::new(event(failed_payload())))
        .await
        .expect("the failure event is queued");
    assert!(logger.async_initialized.load(Ordering::SeqCst));
    assert_eq!(tasks.len(), 1);
    assert_eq!(logger.sink.queued_depth(), 1);
    stop(&logger, &tasks).await;
}

#[tokio::test(start_paused = true)]
async fn posthog_async_init_periodic_flush_sends_the_queue() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    logger
        .async_log_success_event(Arc::new(event(payload())))
        .await
        .expect("the event is queued");
    assert!(transport.requests().is_empty());
    yield_now().await;
    advance(Duration::from_secs(5)).await;
    let flushed = wall_timeout(Duration::from_secs(1), async {
        while transport.requests().is_empty() {
            yield_now().await;
        }
    })
    .await;
    flushed.expect("the periodic flush sends the queue");
    assert_eq!(logger.sink.queued_depth(), 0);
    stop(&logger, &tasks).await;
}
