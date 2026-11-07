#![cfg(test)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;

use super::fixture::{
    BrokenResourceError, FakeRuntime, Script, ScriptedFailure, Step, capture, lock, session_client,
};
use crate::api::mcp_client_resources::operations::McpResourceOperations as _;
use crate::api::mcp_client_resources::settings::ResourceSettings;
use crate::api::mcp_client_resources::types::{
    Capability, Resource, ResourcePage, ServerCapabilities,
};

fn settings() -> ResourceSettings {
    ResourceSettings {
        listing_timeout: Duration::from_secs(30),
    }
}

fn page(names: &[&str], next: Option<&str>) -> ResourcePage {
    ResourcePage {
        resources: names
            .iter()
            .map(|name| Resource {
                uri: format!("test://{name}"),
                name: (*name).to_owned(),
                description: None,
                mime_type: None,
            })
            .collect(),
        next_cursor: next.map(str::to_owned),
    }
}

fn script_with(pages: Vec<Step<ResourcePage>>) -> Script {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: Some(Capability::default()),
        resources: Some(Capability::default()),
    });
    script.resources = VecDeque::from(pages);
    script
}

#[tokio::test]
async fn an_absent_resources_capability_returns_without_calling_the_server() {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: Some(Capability::default()),
        resources: None,
    });
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let resources = client.list_resources(&settings(), false).await.unwrap();
    assert!(resources.is_empty());
    assert!(lock(&shared).cursors.is_empty());
}

#[tokio::test]
async fn resource_pages_are_collected_in_order() {
    let script = script_with(vec![
        Step::Ready(page(&["one"], Some("second"))),
        Step::Ready(page(&["two"], None)),
    ]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let resources = client.list_resources(&settings(), false).await.unwrap();
    let names: Vec<&str> = resources
        .iter()
        .map(|resource| resource.name.as_str())
        .collect();
    assert_eq!(names, ["one", "two"]);
    assert_eq!(lock(&shared).cursors, vec![None, Some("second".to_owned())]);
}

#[tokio::test]
async fn a_broken_stream_is_swallowed_for_resources() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: None,
        resources: Some(Capability::default()),
    });
    script.resources = VecDeque::from([Step::Plain("stream closed")]);
    let runtime = FakeRuntime::<BrokenResourceError>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let resources = client.list_resources(&settings(), false).await.unwrap();
    assert!(resources.is_empty());
    assert!(captured.text().contains("list_resources failed"));
    assert!(captured.text().contains("broken connection/stream"));
}

#[tokio::test]
async fn raising_on_error_returns_resource_failures() {
    let script = script_with(vec![Step::Plain("session lost")]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client.list_resources(&settings(), true).await.unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)));
}
