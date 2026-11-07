#![cfg(test)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;

use super::fixture::{FakeRuntime, Script, ScriptedFailure, Step, capture, lock, session_client};
use crate::api::mcp_client_resources::operations::McpResourceOperations as _;
use crate::api::mcp_client_resources::settings::ResourceSettings;
use crate::api::mcp_client_resources::types::{
    Capability, ResourceTemplate, ResourceTemplatePage, ServerCapabilities,
};

fn settings() -> ResourceSettings {
    ResourceSettings {
        listing_timeout: Duration::from_secs(30),
    }
}

fn page(names: &[&str], next: Option<&str>) -> ResourceTemplatePage {
    ResourceTemplatePage {
        resource_templates: names
            .iter()
            .map(|name| ResourceTemplate {
                uri_template: format!("test://{name}/{{query}}"),
                name: (*name).to_owned(),
                description: None,
                mime_type: None,
            })
            .collect(),
        next_cursor: next.map(str::to_owned),
    }
}

#[tokio::test]
async fn templates_require_the_resources_capability() {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: Some(Capability::default()),
        resources: None,
    });
    script.templates = VecDeque::from([Step::Ready(page(&["one"], None))]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let shared = Arc::clone(&runtime.script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let templates = client
        .list_resource_templates(&settings(), true)
        .await
        .unwrap();
    assert!(templates.is_empty());
    assert!(lock(&shared).cursors.is_empty());
}

#[tokio::test]
async fn template_pages_are_collected_in_order() {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: None,
        resources: Some(Capability::default()),
    });
    script.templates = VecDeque::from([
        Step::Ready(page(&["one"], Some("second"))),
        Step::Ready(page(&["two"], None)),
    ]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let templates = client
        .list_resource_templates(&settings(), false)
        .await
        .unwrap();
    let names: Vec<&str> = templates
        .iter()
        .map(|template| template.name.as_str())
        .collect();
    assert_eq!(names, ["one", "two"]);
}

#[tokio::test]
async fn template_failures_are_swallowed_without_raising() {
    let (captured, _guard) = capture();
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: None,
        resources: Some(Capability::default()),
    });
    script.templates = VecDeque::from([Step::Plain("session lost")]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let templates = client
        .list_resource_templates(&settings(), false)
        .await
        .unwrap();
    assert!(templates.is_empty());
    assert!(captured.text().contains("list_resource_templates failed"));
}

#[tokio::test]
async fn raising_on_error_returns_template_failures() {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: None,
        resources: Some(Capability::default()),
    });
    script.templates = VecDeque::from([Step::Plain("session lost")]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let error = client
        .list_resource_templates(&settings(), true)
        .await
        .unwrap_err();
    assert!(matches!(error, SessionError::Runtime(_)));
}
