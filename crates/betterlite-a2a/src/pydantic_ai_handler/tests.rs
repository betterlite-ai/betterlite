#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use super::{HandlerError, HandlerRequest, PydanticAIHandler};
use crate::pydantic_ai_submit::DEFAULT_TIMEOUT;
use crate::pydantic_ai_submit::error::SubmitError;
use crate::pydantic_ai_submit::tests::{Counter, ScriptExhausted, ScriptedTransport, json};
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::send::PydanticAITransformation;

const COMPLETED: &str = r#"{"result": {"id": "task-1", "status": {"state": "completed"}, "history": [
    {"role": "agent", "messageId": "answer", "parts": [{"kind": "text", "text": "hello"}]}
]}}"#;

fn handler(bodies: &[&str]) -> PydanticAIHandler<ScriptedTransport> {
    PydanticAIHandler {
        transformation: PydanticAITransformation {
            transport: ScriptedTransport::answering(bodies),
        },
    }
}

fn request<'request>(
    params: &'request AgentJson,
    headers: &'request BTreeMap<String, String>,
    api_base: Option<&'request str>,
) -> HandlerRequest<'request, AgentJson> {
    HandlerRequest {
        request_id: "req-1",
        params,
        api_base,
        timeout: DEFAULT_TIMEOUT,
        headers,
    }
}

#[tokio::test]
async fn pydantic_ai_handler_answers_with_the_transformed_message() {
    let handler = handler(&[COMPLETED]);
    let (params, headers) = (json(r#"{"message": {"parts": []}}"#), BTreeMap::new());
    let event = handler
        .handle_non_streaming(
            &request(&params, &headers, Some("http://agent.test")),
            &mut Counter(0),
        )
        .await
        .unwrap();
    assert_eq!(event.result.parts[0].text.as_deref(), Some("hello"));
    assert_eq!(event.result.message_id, "answer");
}

#[tokio::test]
async fn pydantic_ai_handler_streams_the_simulated_events() {
    let handler = handler(&[COMPLETED]);
    let (params, headers) = (json(r#"{"message": {"parts": []}}"#), BTreeMap::new());
    let events = handler
        .handle_streaming(
            &request(&params, &headers, Some("http://agent.test")),
            StreamPacing::default(),
            &mut Counter(0),
        )
        .await
        .unwrap();
    assert!(!events.is_empty());
}

#[tokio::test]
async fn pydantic_ai_handler_requires_an_api_base() {
    let handler = handler(&[]);
    let (params, headers) = (json("{}"), BTreeMap::new());
    let plain = handler
        .handle_non_streaming(&request(&params, &headers, None), &mut Counter(0))
        .await
        .unwrap_err();
    let streaming = handler
        .handle_streaming(
            &request(&params, &headers, None),
            StreamPacing::default(),
            &mut Counter(0),
        )
        .await
        .unwrap_err();
    assert!(matches!(plain, HandlerError::ApiBaseRequired));
    assert!(matches!(streaming, HandlerError::ApiBaseRequired));
    assert!(handler.transformation.transport.recorded().is_empty());
}

#[tokio::test]
async fn pydantic_ai_handler_forwards_headers_and_surfaces_submit_errors() {
    let handler = handler(&[]);
    let params = json("{}");
    let headers = BTreeMap::from([("X-Trace".to_owned(), "t-1".to_owned())]);
    let error = handler
        .handle_non_streaming(
            &request(&params, &headers, Some("http://agent.test")),
            &mut Counter(0),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        HandlerError::Submit(SubmitError::Transport(ScriptExhausted))
    ));
    let recorded = handler.transformation.transport.recorded();
    assert_eq!(recorded[0].headers["X-Trace"], "t-1");
}
