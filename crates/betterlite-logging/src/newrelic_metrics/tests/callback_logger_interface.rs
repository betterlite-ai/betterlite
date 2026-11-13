use std::sync::Arc;

use super::support::{Fixture, event, payload};
use crate::callback_logger::CallbackLogger;
use crate::newrelic_metrics::logger::NewRelicMetricsLogger;
use crate::newrelic_metrics::queue::queue_length;

#[tokio::test]
async fn callback_logger_boxes_as_a_callback_logger_named_newrelic() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    let concrete: Arc<NewRelicMetricsLogger> = Arc::clone(&logger);
    let loggers: Vec<Arc<dyn CallbackLogger>> = vec![concrete];
    let boxed = loggers.first().expect("one boxed logger");
    assert_eq!(boxed.callback_name(), "newrelic");
    boxed
        .async_log_success_event(event(payload()))
        .await
        .expect("a logged success event");
    assert_eq!(queue_length(&logger), 1);
}

#[tokio::test]
async fn callback_logger_success_events_queue_a_metric_record() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("a logged success event");
    assert_eq!(queue_length(&logger), 1);
    assert!(fixture.transport.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_failure_events_queue_a_metric_record() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    logger
        .async_log_failure_event(event(payload()))
        .await
        .expect("a logged failure event");
    assert_eq!(queue_length(&logger), 1);
    assert!(fixture.transport.requests().is_empty());
}
