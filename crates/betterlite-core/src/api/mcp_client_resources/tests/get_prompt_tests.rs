#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;
use serde_json::Map;

use super::fixture::{
    BrokenResourceError, FakeRuntime, Script, ScriptedFailure, Step, capture, lock, session_client,
};
use crate::api::mcp_client_resources::operations::McpResourceOperations as _;
use crate::api::mcp_client_resources::types::{GetPromptRequestParams, GetPromptResult};

fn result() -> GetPromptResult {
    GetPromptResult {
        description: Some("greeting".to_owned()),
        messages: vec![
            serde_json::from_str(r#"{"role":"user","content":{"type":"text","text":"hello"}}"#)
                .unwrap(),
        ],
        extra: Map::new(),
    }
}

fn params() -> GetPromptRequestParams {
    let mut arguments = BTreeMap::new();
    arguments.insert("who".to_owned(), "world".to_owned());
    GetPromptRequestParams {
        name: "greet".to_owned(),
        arguments: Some(arguments),
    }
}

#[tokio::test]
async fn the_request_name_and_arguments_reach_the_session() {
    let mut script = Script::with_capabilities(None);
    script.prompt_result = Some(Step::Ready(result()));
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let outline = client.get_prompt(params()).await.unwrap();
    assert_eq!(outline.messages.len(), 1);
    let recorded = lock(&shared).prompt_params.clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].name, "greet");
    assert_eq!(
        recorded[0]
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("who"))
            .map(String::as_str),
        Some("world")
    );
}

#[tokio::test]
async fn a_failure_keeps_its_cause_and_logs_the_traceback() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(None);
    script.prompt_result = Some(Step::Plain("prompt exploded"));
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client.get_prompt(params()).await.unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)));
    let text = captured.text();
    assert!(text.contains("traceback"));
    assert!(text.contains("get_prompt failed"));
    assert!(text.contains("prompt exploded"));
    assert!(text.contains("WARN"));
    assert!(text.contains("Server: https://mcp.example.com/mcp"));
}

#[tokio::test]
async fn a_broken_stream_adds_the_connection_diagnostic() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(None);
    script.prompt_result = Some(Step::Plain("stream closed"));
    let runtime = FakeRuntime::<BrokenResourceError>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let _error = client.get_prompt(params()).await.unwrap_err();
    assert!(captured.text().contains("broken connection/stream"));
}
