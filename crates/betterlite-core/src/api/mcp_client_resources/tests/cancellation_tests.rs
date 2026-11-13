#![cfg(test)]

use std::collections::VecDeque;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;
use url::Url;

use super::fixture::{FakeRuntime, Script, ScriptedFailure, Step, capture, session_client};
use crate::api::mcp_client_resources::operations::McpResourceOperations as _;
use crate::api::mcp_client_resources::settings::ResourceSettings;
use crate::api::mcp_client_resources::types::{
    GetPromptRequestParams, PromptPage, ServerCapabilities,
};

fn url() -> Url {
    Url::parse("test://one").unwrap()
}

fn settings() -> ResourceSettings {
    ResourceSettings {
        listing_timeout: Duration::from_secs(30),
    }
}

fn page() -> PromptPage {
    PromptPage {
        prompts: Vec::new(),
        next_cursor: None,
    }
}

#[tokio::test]
async fn a_cancelled_list_propagates_with_a_warning() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(Some(ServerCapabilities::default()));
    script.prompts = VecDeque::from(vec![Step::Ready(page())]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    client.caller_cancelled.cancel();
    let error = client.list_prompts(&settings(), false).await.unwrap_err();
    assert!(matches!(error, SessionError::Cancelled));
    assert!(captured.text().contains("list_prompts was cancelled"));
}

#[tokio::test]
async fn raise_on_error_does_not_swallow_cancellation() {
    let runtime = FakeRuntime::<ScriptedFailure>::new(Script::default());
    let mut client = session_client(runtime, Duration::from_secs(60));
    client.caller_cancelled.cancel();
    let error = client.list_resources(&settings(), true).await.unwrap_err();
    assert!(matches!(error, SessionError::Cancelled));
}

#[tokio::test]
async fn a_cancelled_prompt_fetch_propagates_with_a_warning() {
    let (captured, _guard) = capture();
    let runtime = FakeRuntime::<ScriptedFailure>::new(Script::default());
    let mut client = session_client(runtime, Duration::from_secs(60));
    client.caller_cancelled.cancel();
    let params = GetPromptRequestParams {
        name: "greet".to_owned(),
        arguments: None,
    };
    let error = client.get_prompt(params).await.unwrap_err();
    assert!(matches!(error, SessionError::Cancelled));
    assert!(captured.text().contains("get_prompt was cancelled"));
}

#[tokio::test]
async fn a_cancelled_resource_read_propagates_with_a_warning() {
    let (captured, _guard) = capture();
    let runtime = FakeRuntime::<ScriptedFailure>::new(Script::default());
    let mut client = session_client(runtime, Duration::from_secs(60));
    client.caller_cancelled.cancel();
    let error = client.read_resource(url()).await.unwrap_err();
    assert!(matches!(error, SessionError::Cancelled));
    assert!(captured.text().contains("read_resource was cancelled"));
}
