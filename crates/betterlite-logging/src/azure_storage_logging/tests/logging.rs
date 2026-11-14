#![cfg(test)]

use super::support::{ENTRA, event, fixture};
use crate::callback_logger::CallbackLogger as _;

#[tokio::test]
async fn a_success_event_queues_the_standard_logging_object() {
    let fixture = fixture(&ENTRA, &[]);
    fixture
        .logger
        .async_log_success_event(event("id-1"))
        .await
        .expect("the event is queued");
    let queued = fixture.logger.queue.pending();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued.first().map(|record| record.id.as_str()),
        Some("id-1")
    );
}

#[tokio::test]
async fn a_failure_event_queues_the_same_way() {
    let fixture = fixture(&ENTRA, &[]);
    fixture
        .logger
        .async_log_failure_event(event("id-2"))
        .await
        .expect("the event is queued");
    let queued = fixture.logger.queue.pending();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued.first().map(|record| record.id.as_str()),
        Some("id-2")
    );
}

#[tokio::test]
async fn both_event_kinds_share_one_queue() {
    let fixture = fixture(&ENTRA, &[]);
    fixture
        .logger
        .async_log_success_event(event("id-1"))
        .await
        .expect("queued");
    fixture
        .logger
        .async_log_failure_event(event("id-2"))
        .await
        .expect("queued");
    assert_eq!(fixture.logger.queue.pending().len(), 2);
}
