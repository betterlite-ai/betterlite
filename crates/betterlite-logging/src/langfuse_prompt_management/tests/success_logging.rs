use super::support;
use crate::callback_logger::CallbackLogger;

#[tokio::test]
async fn the_success_span_carries_the_messages_and_the_response() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let payload = support::success_payload(Some(support::CALL_ID));
    let messages = payload.messages.clone();
    fixture
        .manager
        .async_log_success_event(support::event(payload))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.level").as_deref(),
        Some("DEFAULT")
    );
    let input = support::langfuse::object(support::langfuse::attribute_json(
        &spans,
        "langfuse.observation.input",
    ));
    assert_eq!(input.get("messages"), messages.as_ref());
    let output = support::langfuse::attribute_text(&spans, "langfuse.observation.output")
        .expect("the response is projected as the output");
    assert!(output.contains("hi"), "the output was {output}");
}

#[tokio::test]
async fn the_success_span_carries_the_model_parameters() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_success_event(support::event(support::success_payload(None)))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    let parameters = support::langfuse::object(support::langfuse::attribute_json(
        &spans,
        "langfuse.observation.model.parameters",
    ));
    assert!(parameters.contains_key("temperature"));
}
