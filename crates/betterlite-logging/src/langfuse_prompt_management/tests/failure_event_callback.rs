use super::support;
use crate::callback_logger::CallbackLogger;

#[tokio::test]
async fn the_sync_failure_event_reaches_the_logger_through_the_bridge() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let payload = support::failure_payload(Some(support::CALL_ID), Some("boom!"));
    fixture
        .manager
        .log_failure_event(&payload)
        .expect("the bridged call returns the async result");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.level").as_deref(),
        Some("ERROR")
    );
    let trace = fixture
        .manager
        .trace_cache
        .get_cache(support::CALL_ID, "langfuse")
        .expect("the trace cache reads");
    assert_eq!(trace.as_deref(), Some(support::TRACE_ID));
}
