#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use super::{HandlerError, HandlerRequest, PydanticAIHandler};
use crate::pydantic_ai_submit::DEFAULT_TIMEOUT;
use crate::pydantic_ai_submit::error::SubmitError;
use crate::pydantic_ai_submit::tests::{Counter, ScriptExhausted, ScriptedTransport, json};
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::send::{
    ProjectedMessage, PydanticAITransformation, ReplayedStream,
};

const COMPLETED: &str = r#"{"result": {"id": "task-1", "status": {"state": "completed"}, "history": [
    {"role": "agent", "messageId": "answer", "parts": [{"kind": "text", "text": "hello"}]}
]}}"#;

fn handler(bodies: &[&str]) -> PydanticAIHandler<ScriptedTransport, Counter> {
    PydanticAIHandler {
        transformation: PydanticAITransformation {
            transport: ScriptedTransport::answering(bodies),
            source: Counter(0),
        },
    }
}

fn request<'request>(
    params: &'request AgentJson,
    headers: &'request BTreeMap<String, String>,
    api_base: Option<&'request str>,
) -> HandlerRequest<'request> {
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
    let mut handler = handler(&[COMPLETED]);
    let (params, headers) = (json(r#"{"message": {"parts": []}}"#), BTreeMap::new());
    let event = handler
        .handle(
            &request(&params, &headers, Some("http://agent.test")),
            ProjectedMessage,
        )
        .await
        .unwrap();
    assert_eq!(event.result.parts[0].text.as_deref(), Some("hello"));
    assert_eq!(event.result.message_id, "answer");
}

#[tokio::test]
async fn pydantic_ai_handler_streams_the_simulated_events() {
    let mut handler = handler(&[COMPLETED]);
    let (params, headers) = (json(r#"{"message": {"parts": []}}"#), BTreeMap::new());
    let events = handler
        .handle(
            &request(&params, &headers, Some("http://agent.test")),
            ReplayedStream(StreamPacing::default()),
        )
        .await
        .unwrap();
    let values: Vec<AgentJson> = events
        .iter()
        .map(|step| serde_json::to_value(&step.event).unwrap())
        .collect();
    assert!(values.iter().all(|event| event["id"] == "req-1"));
    let shape: Vec<_> = values
        .iter()
        .map(|event| {
            (
                event["result"]["kind"].as_str().unwrap(),
                event["result"]["status"]["state"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("task", Some("submitted")),
            ("status-update", Some("working")),
            ("artifact-update", None),
            ("status-update", Some("completed")),
        ]
    );
    assert_eq!(values[2]["result"]["artifact"]["parts"][0]["text"], "hello");
}

#[tokio::test]
async fn pydantic_ai_handler_requires_an_api_base() {
    let mut handler = handler(&[]);
    let (params, headers) = (json("{}"), BTreeMap::new());
    let plain = handler
        .handle(&request(&params, &headers, None), ProjectedMessage)
        .await
        .unwrap_err();
    let streaming = handler
        .handle(
            &request(&params, &headers, None),
            ReplayedStream(StreamPacing::default()),
        )
        .await
        .unwrap_err();
    assert!(matches!(plain, HandlerError::ApiBaseRequired));
    assert!(matches!(streaming, HandlerError::ApiBaseRequired));
    assert!(handler.transformation.transport.recorded().is_empty());
}

#[tokio::test]
async fn pydantic_ai_handler_forwards_headers_and_surfaces_submit_errors() {
    let mut handler = handler(&[]);
    let params = json("{}");
    let headers = BTreeMap::from([("X-Trace".to_owned(), "t-1".to_owned())]);
    let error = handler
        .handle(
            &request(&params, &headers, Some("http://agent.test")),
            ProjectedMessage,
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
