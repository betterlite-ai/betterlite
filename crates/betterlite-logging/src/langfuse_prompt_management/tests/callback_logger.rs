use super::support;
use crate::callback_logger::CallbackLogger;

#[tokio::test]
async fn callback_logger_serves_success_and_failure_events_for_langfuse() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    assert_eq!(fixture.manager.callback_name(), "langfuse");
    assert!(!fixture.manager.turn_off_message_logging());
    assert!(!fixture.manager.redacts_messages_itself());
    assert!(fixture.manager.message_logging());
    let success = support::event(support::success_payload(Some(support::CALL_ID)));
    fixture
        .manager
        .async_log_success_event(success)
        .await
        .expect("the success event logs");
    let failure = support::event(support::failure_payload(None, Some("boom!")));
    fixture
        .manager
        .async_log_failure_event(failure)
        .await
        .expect("the failure event logs");
    assert!(fixture.manager.logger.flush().await);
    assert_eq!(fixture.spans.spans().len(), 2);
}

#[tokio::test]
async fn callback_logger_ignores_a_stream_event_like_litellms_custom_logger() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let stream = support::event(support::success_payload(Some(support::CALL_ID)));
    assert!(fixture.manager.async_log_stream_event(stream).await.is_ok());
    assert!(fixture.manager.logger.flush().await);
    assert!(fixture.spans.spans().is_empty());
}
