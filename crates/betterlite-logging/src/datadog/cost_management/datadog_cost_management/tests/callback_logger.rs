use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{self, Fixture};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;

fn event(payload: Arc<StandardLoggingPayload>) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload,
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

#[tokio::test]
async fn callback_logger_reports_its_name() {
    let logger = Fixture::new(&[]).build();
    assert_eq!(logger.callback_name(), "datadog_cost_management");
}

#[tokio::test]
async fn callback_logger_success_events_queue_a_positive_cost() {
    let fixture = Fixture::new(&[]);
    let logger = fixture.build();
    logger
        .async_log_success_event(event(support::default_payload().build()))
        .await
        .expect("the event is recorded");
    assert_eq!(logger.queue_length(), 1);
}

#[tokio::test]
async fn callback_logger_success_events_drop_a_zero_cost() {
    let fixture = Fixture::new(&[]);
    let logger = fixture.build();
    let zero = support::default_payload()
        .set("/response_cost", support::number(0.0))
        .build();
    logger
        .async_log_success_event(event(zero))
        .await
        .expect("the event is recorded");
    assert_eq!(logger.queue_length(), 0);
}

#[tokio::test]
async fn callback_logger_failure_events_do_not_queue() {
    let fixture = Fixture::new(&[]);
    let logger = fixture.build();
    logger
        .async_log_failure_event(event(support::default_payload().build()))
        .await
        .expect("the failure event is ignored");
    assert_eq!(logger.queue_length(), 0);
}

#[tokio::test]
async fn callback_logger_drops_the_oldest_record_past_the_queue_cap() {
    let mut init = support::init(&[]);
    init.max_queue_size = Some(1);
    let fixture = Fixture::new(&[]).with_init(init);
    let logger = fixture.build();
    logger
        .async_log_success_event(event(support::default_payload().build()))
        .await
        .expect("the first event is recorded");
    let second = support::default_payload()
        .set("/id", support::text("chatcmpl-2"))
        .build();
    logger
        .async_log_success_event(event(second))
        .await
        .expect("the second event is recorded");
    assert_eq!(logger.queue_length(), 1);
    let pending = logger.queue.pending();
    assert_eq!(
        pending.first().map(|payload| payload.id.as_str()),
        Some("chatcmpl-2")
    );
}

#[tokio::test]
async fn callback_logger_success_events_flush_at_the_batch_size() {
    let mut init = support::init(&[]);
    init.batch_size = Some(1);
    let fixture = Fixture::new(&[202]).with_init(init);
    let logger = fixture.build();
    logger
        .async_log_success_event(event(support::default_payload().build()))
        .await
        .expect("the event is recorded");
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(logger.queue_length(), 0);
}
