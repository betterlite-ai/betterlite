use std::iter::repeat_with;
use std::sync::Arc;

use secrecy::ExposeSecret as _;

use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::posthog_logging::logger::PostHogLogger;
use crate::posthog_logging::tests::support::{
    ScriptedTransport, default_settings, dynamic, event, event_with, field, fixed_id, list_len,
    logger_with, payload, stop,
};

async fn queue(logger: &PostHogLogger, events: &[LoggerEvent]) {
    for event in events {
        logger
            .async_log_success_event(Arc::new(event.clone()))
            .await
            .expect("the event is queued");
    }
}

fn repeated_events(count: usize) -> Vec<LoggerEvent> {
    repeat_with(|| event(payload())).take(count).collect()
}

#[tokio::test]
async fn posthog_async_queue_keeps_each_events_credentials() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let first = event_with(
        payload(),
        dynamic(&[
            ("posthog_api_key", "first-key"),
            ("posthog_api_url", "https://first.example/"),
        ]),
    );
    let second = event(payload());
    queue(&logger, &[first, second]).await;
    let queued = logger.sink.queue.pending();
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].api_key.expose_secret(), "first-key");
    assert_eq!(queued[0].api_url, "https://first.example/");
    assert_eq!(queued[1].api_key.expose_secret(), "phk-test");
    assert_eq!(queued[1].api_url, "https://us.i.posthog.com");
    assert!(transport.requests().is_empty());
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_queue_flushes_at_the_hundredth_event() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let events = repeated_events(100);
    queue(&logger, &events).await;
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(list_len(field(&requests[0].body, "batch")), 100);
    assert_eq!(logger.sink.queued_depth(), 0);
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_queue_keeps_ninety_nine_events() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let events = repeated_events(99);
    queue(&logger, &events).await;
    assert!(transport.requests().is_empty());
    assert_eq!(logger.sink.queued_depth(), 99);
    stop(&logger, &tasks).await;
}
