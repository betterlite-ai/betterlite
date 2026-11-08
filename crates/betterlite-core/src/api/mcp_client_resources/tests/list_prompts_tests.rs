#![cfg(test)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::stream::StreamFailure;
use serde_json::Map;

use super::fixture::{
    BrokenResourceError, FakeRuntime, Script, ScriptedFailure, Step, capture, lock, protocol,
    session_client,
};
use crate::api::mcp_client_resources::failure::is_broken_stream;
use crate::api::mcp_client_resources::operations::McpResourceOperations as _;
use crate::api::mcp_client_resources::pagination::METHOD_NOT_FOUND;
use crate::api::mcp_client_resources::settings::ResourceSettings;
use crate::api::mcp_client_resources::types::{Capability, Prompt, PromptPage, ServerCapabilities};

fn settings() -> ResourceSettings {
    ResourceSettings {
        listing_timeout: Duration::from_secs(30),
    }
}

fn page(names: &[&str], next: Option<&str>) -> PromptPage {
    PromptPage {
        prompts: names
            .iter()
            .map(|name| Prompt {
                name: (*name).to_owned(),
                description: None,
                arguments: None,
                extra: Map::new(),
            })
            .collect(),
        next_cursor: next.map(str::to_owned),
    }
}

fn script_with(pages: Vec<Step<PromptPage>>) -> Script {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: Some(Capability::default()),
        resources: Some(Capability::default()),
    });
    script.prompts = VecDeque::from(pages);
    script
}

#[tokio::test]
async fn an_absent_prompts_capability_returns_without_calling_the_server() {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: None,
        resources: Some(Capability::default()),
    });
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let prompts = client.list_prompts(&settings(), false).await.unwrap();
    assert!(prompts.is_empty());
    assert!(lock(&shared).cursors.is_empty());
}

#[tokio::test]
async fn an_unknown_capability_set_still_calls_the_server() {
    let mut script = Script::with_capabilities(None);
    script.prompts = VecDeque::from([Step::Ready(page(&["one"], None))]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let prompts = client.list_prompts(&settings(), false).await.unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(lock(&shared).cursors, vec![None]);
}

#[tokio::test]
async fn every_prompt_page_is_collected_in_cursor_order() {
    let script = script_with(vec![
        Step::Ready(page(&["one"], Some("second"))),
        Step::Ready(page(&["two"], Some("third"))),
        Step::Ready(page(&["three"], None)),
    ]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let prompts = client.list_prompts(&settings(), false).await.unwrap();
    let names: Vec<&str> = prompts.iter().map(|prompt| prompt.name.as_str()).collect();
    assert_eq!(names, ["one", "two", "three"]);
    assert_eq!(
        lock(&shared).cursors,
        vec![None, Some("second".to_owned()), Some("third".to_owned())]
    );
}

#[tokio::test]
async fn a_first_page_method_not_found_returns_an_empty_list() {
    let (captured, _guard) = capture();
    let script = script_with(vec![Step::Protocol(protocol(
        METHOD_NOT_FOUND,
        "unsupported",
    ))]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let prompts = client.list_prompts(&settings(), true).await.unwrap();
    assert!(prompts.is_empty());
    assert!(captured.text().contains("unsupported"));
}

#[tokio::test]
async fn a_later_page_method_not_found_fails_the_walk() {
    let script = script_with(vec![
        Step::Ready(page(&["one"], Some("second"))),
        Step::Protocol(protocol(METHOD_NOT_FOUND, "changed")),
    ]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client.list_prompts(&settings(), true).await.unwrap_err();
    match error {
        SessionError::Protocol(failure) => assert_eq!(failure.code, METHOD_NOT_FOUND),
        other @ (SessionError::Runtime(_)
        | SessionError::Stream(_)
        | SessionError::Cancelled
        | SessionError::CleanupTimedOut
        | SessionError::CleanupTask(_)
        | SessionError::ReadTimeout(_)
        | SessionError::Configuration(_)) => panic!("expected the relayed failure, got {other:?}"),
    }
}

#[tokio::test]
async fn another_first_page_protocol_error_fails() {
    let script = script_with(vec![Step::Protocol(protocol(-32_603, "internal"))]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client.list_prompts(&settings(), true).await.unwrap_err();
    assert!(matches!(error, SessionError::Protocol(_)));
}

#[tokio::test]
async fn a_broken_stream_is_swallowed_and_reported_without_raising() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: Some(Capability::default()),
        resources: Some(Capability::default()),
    });
    script.prompts = VecDeque::from([Step::Plain("stream closed")]);
    let runtime = FakeRuntime::<BrokenResourceError>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let prompts = client.list_prompts(&settings(), false).await.unwrap();
    assert!(prompts.is_empty());
    let text = captured.text();
    assert!(text.contains("list_prompts failed"));
    assert!(text.contains("broken connection/stream"));
}

#[tokio::test]
async fn raising_on_error_returns_the_failure_without_an_error_line() {
    let (captured, _guard) = capture();
    let script = script_with(vec![Step::Plain("session lost")]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client.list_prompts(&settings(), true).await.unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)));
    assert!(!captured.text().contains("list_prompts failed"));
}

#[tokio::test]
async fn a_transport_failure_logs_a_warning_from_the_session_runner() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(Some(ServerCapabilities {
        prompts: Some(Capability::default()),
        resources: None,
    }));
    script.transport_fails = true;
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client.list_prompts(&settings(), true).await.unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)));
    let text = captured.text();
    assert!(text.contains("WARN"));
    assert!(text.contains("https://mcp.example.com/mcp"));
}

#[test]
fn stream_failures_and_broken_type_names_count_as_broken_streams() {
    let stream: SessionError<ScriptedFailure> = SessionError::Stream(Arc::new(
        StreamFailure::Http(Arc::new(ScriptedFailure::Plain("closed"))),
    ));
    assert!(is_broken_stream(&stream));
    let broken: SessionError<BrokenResourceError> =
        SessionError::Runtime(Arc::new(BrokenResourceError));
    assert!(is_broken_stream(&broken));
    let plain: SessionError<ScriptedFailure> =
        SessionError::Runtime(Arc::new(ScriptedFailure::Plain("plain")));
    assert!(!is_broken_stream(&plain));
}
