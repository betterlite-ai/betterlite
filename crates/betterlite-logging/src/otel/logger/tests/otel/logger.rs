#![cfg(test)]

mod lifecycle;

use betterlite_logging::callback_logger::CallbackLogger;

use crate::otel::support::{
    anchored, event, failure, finished, harness, inference, inference_failure, names, owned_http,
    success, with_id,
};

#[tokio::test]
async fn callback_logger_exposes_the_otel_callback_name() {
    let harness = harness(owned_http(), anchored());
    let callback: &dyn CallbackLogger = &harness.logger;
    assert_eq!(callback.callback_name(), "arize");
    assert!(callback.message_logging());
    assert!(!callback.turn_off_message_logging());
    assert!(!callback.redacts_messages_itself());
}

#[tokio::test]
async fn callback_logger_lifecycle_runs_on_success_and_failure_events() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    failure(
        &harness.logger,
        event(with_id(inference_failure(), "call-b")),
    )
    .await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 2);
    assert!(names(&spans).iter().all(|name| name.contains("gpt-4o")));
}
