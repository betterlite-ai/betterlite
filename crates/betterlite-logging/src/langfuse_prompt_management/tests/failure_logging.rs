use super::support;
use crate::callback_logger::CallbackLogger;

#[tokio::test]
async fn the_failure_span_takes_the_status_message_from_the_error_string() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_failure_event(support::event(support::failure_payload(
            Some(support::CALL_ID),
            Some("boom!"),
        )))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.level").as_deref(),
        Some("ERROR")
    );
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.output").as_deref(),
        Some("boom!")
    );
}

#[tokio::test]
async fn a_failure_without_an_error_string_reports_unknown_error() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_failure_event(support::event(support::failure_payload(None, None)))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.output").as_deref(),
        Some("Unknown error")
    );
}

#[tokio::test]
async fn an_empty_error_string_also_reports_unknown_error() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_failure_event(support::event(support::failure_payload(None, Some(""))))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.output").as_deref(),
        Some("Unknown error")
    );
}
