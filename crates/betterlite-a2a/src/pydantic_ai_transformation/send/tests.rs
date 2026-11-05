#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use super::PydanticAITransformation;
use crate::pydantic_ai_submit::error::SubmitError;
use crate::pydantic_ai_submit::tests::{Counter, ScriptedTransport, json};
use crate::pydantic_ai_submit::{AgentCall, DEFAULT_TIMEOUT};
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;

const SUBMITTED: &str = r#"{"result": {"id": "task-1", "status": {"state": "submitted"}}}"#;
const WORKING: &str = r#"{"result": {"id": "task-1", "status": {"state": "working"}}}"#;
const COMPLETED: &str = r#"{"result": {"id": "task-1", "status": {"state": "completed"}, "history": [
    {"role": "user", "messageId": "question", "parts": [{"kind": "text", "text": "q"}]},
    {"role": "agent", "messageId": "answer", "parts": [{"kind": "text", "text": "polled answer"}]}
]}}"#;

fn call<'call>(
    params: &'call AgentJson,
    headers: &'call BTreeMap<String, String>,
) -> AgentCall<'call, AgentJson> {
    AgentCall {
        api_base: "http://agent.test/",
        request_id: "req-1",
        params,
        timeout: DEFAULT_TIMEOUT,
        headers,
    }
}

fn params() -> AgentJson {
    json(r#"{"message": {"role": "user", "parts": []}}"#)
}

#[tokio::test]
async fn pydantic_ai_submit_non_streaming_transforms_the_polled_task() {
    let transformation = PydanticAITransformation {
        transport: ScriptedTransport::answering(&[SUBMITTED, WORKING, COMPLETED]),
    };
    let (params, headers) = (params(), BTreeMap::new());
    let event = transformation
        .send_non_streaming_request(&call(&params, &headers), &mut Counter(0))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"message","role":"agent","parts":[{"kind":"text","text":"polled answer"}],"messageId":"answer"}}"#
    );
    assert_eq!(transformation.transport.pause_count(), 1);
}

#[tokio::test]
async fn pydantic_ai_submit_streaming_replays_the_raw_task() {
    let transformation = PydanticAITransformation {
        transport: ScriptedTransport::answering(&[SUBMITTED, COMPLETED]),
    };
    let (params, headers) = (params(), BTreeMap::new());
    let steps = transformation
        .send_streaming_request(
            &call(&params, &headers),
            StreamPacing::default(),
            &mut Counter(0),
        )
        .await
        .unwrap();
    let events: Vec<_> = steps.iter().map(|step| &step.event).collect();
    let text = serde_json::to_string(&events).unwrap();
    assert!(text.contains("polled answer"));
    assert!(text.contains(r#""id":"req-1""#));
}

#[tokio::test]
async fn pydantic_ai_submit_send_reports_a_task_that_cannot_be_read() {
    let transformation = PydanticAITransformation {
        transport: ScriptedTransport::answering(&[
            r#"{"result": {"id": "", "history": "not a list"}}"#,
        ]),
    };
    let (params, headers) = (params(), BTreeMap::new());
    let error = transformation
        .send_non_streaming_request(&call(&params, &headers), &mut Counter(0))
        .await
        .unwrap_err();
    assert!(matches!(error, SubmitError::Response(_)));
}
