#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;

use super::fixture::{
    BrokenResourceError, FakeRuntime, Script, ScriptedFailure, Step, capture, lock, session_client,
};
use crate::api::mcp_client_resources::operations::McpResourceOperations as _;
use crate::api::mcp_client_resources::types::{ReadResourceResult, ResourceContents};

fn contents() -> ReadResourceResult {
    ReadResourceResult {
        contents: vec![ResourceContents {
            uri: "test://one".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            text: Some("hello".to_owned()),
            blob: None,
        }],
    }
}

#[tokio::test]
async fn the_url_reaches_the_session_and_the_contents_come_back() {
    let mut script = Script::with_capabilities(None);
    script.resource_result = Some(Step::Ready(contents()));
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let read = client.read_resource("test://one".to_owned()).await.unwrap();
    assert_eq!(read.contents.len(), 1);
    assert_eq!(read.contents[0].text.as_deref(), Some("hello"));
    assert_eq!(lock(&shared).resource_uris, vec!["test://one"]);
}

#[tokio::test]
async fn a_failure_keeps_its_cause_and_logs_the_traceback() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(None);
    script.resource_result = Some(Step::Plain("read exploded"));
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client
        .read_resource("test://one".to_owned())
        .await
        .unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)));
    let text = captured.text();
    assert!(text.contains("traceback"));
    assert!(text.contains("read_resource failed"));
    assert!(text.contains("WARN"));
    assert!(text.contains("https://mcp.example.com/mcp"));
}

#[tokio::test]
async fn a_broken_stream_adds_the_connection_diagnostic() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(None);
    script.resource_result = Some(Step::Plain("stream closed"));
    let runtime = FakeRuntime::<BrokenResourceError>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let _error = client
        .read_resource("test://one".to_owned())
        .await
        .unwrap_err();
    assert!(captured.text().contains("broken connection/stream"));
}
