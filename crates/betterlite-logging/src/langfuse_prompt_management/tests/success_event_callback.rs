use super::support;
use crate::callback_logger::CallbackLogger;

#[tokio::test]
async fn the_sync_success_event_reaches_the_logger_through_the_bridge() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let payload = support::success_payload(Some(support::CALL_ID));
    // the calling thread already runs a runtime, so only the bridge can drive the async hook
    fixture
        .manager
        .log_success_event(&payload)
        .expect("the bridged call returns the async result");
    assert!(fixture.manager.logger.flush().await);
    assert_eq!(fixture.spans.spans().len(), 1);
    let trace = fixture
        .manager
        .trace_cache
        .get_cache(support::CALL_ID, "langfuse")
        .expect("the trace cache reads");
    assert_eq!(trace.as_deref(), Some(support::TRACE_ID));
}
