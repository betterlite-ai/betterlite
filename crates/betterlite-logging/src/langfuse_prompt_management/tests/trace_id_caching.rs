use super::support;
use crate::callback_logger::CallbackLogger;

#[tokio::test]
async fn a_logged_event_stores_its_trace_id_under_the_call_id() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let event = support::event(support::success_payload(Some(support::CALL_ID)));
    fixture
        .manager
        .async_log_success_event(event)
        .await
        .expect("the event logs");
    let trace = fixture
        .manager
        .trace_cache
        .get_cache(support::CALL_ID, "langfuse")
        .expect("the trace cache reads");
    assert_eq!(trace.as_deref(), Some(support::TRACE_ID));
}

#[tokio::test]
async fn an_event_without_a_call_id_stores_no_trace_id() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let event = support::event(support::success_payload(None));
    fixture
        .manager
        .async_log_success_event(event)
        .await
        .expect("the event logs");
    assert!(
        fixture
            .manager
            .trace_cache
            .get_cache(support::CALL_ID, "langfuse")
            .expect("the trace cache reads")
            .is_none()
    );
    assert!(fixture.manager.logger.flush().await);
    assert!(!fixture.spans.spans().is_empty());
}
