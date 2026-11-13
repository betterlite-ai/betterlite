#![cfg(test)]

use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::Level;

use super::tests::fixture::{
    CallOutcome, RecordingHost, Script, TestError, build_client, capture, hanging,
};
use crate::client::sessions::error::SessionError;
use crate::client::tools::calling::{CallToolOptions, call_tool};
use crate::client::tools::listing::list_tools;
use crate::tools::tool_calls::CallRequest;

fn request() -> CallRequest {
    CallRequest {
        name: "echo".to_owned(),
        arguments: None,
    }
}

fn swallowing() -> CallToolOptions {
    CallToolOptions {
        raise_on_error: false,
        allow_input_required: false,
    }
}

#[tokio::test]
async fn list_tools_propagates_cancellation() {
    let token = CancellationToken::new();
    token.cancel();
    let mut client = build_client(
        Script::<TestError>::default(),
        Duration::from_secs(60),
        token,
    );
    let (logs, _guard) = capture();
    let outcome = list_tools(&mut client, Duration::from_secs(30), false).await;
    assert!(matches!(outcome, Err(SessionError::Cancelled)));
    assert_eq!(
        logs.matching("MCP client list_tools was cancelled").len(),
        1
    );
    assert_eq!(logs.count(Level::ERROR), 0);
}

#[tokio::test]
async fn call_tool_propagates_cancellation() {
    let token = CancellationToken::new();
    token.cancel();
    let mut client = build_client(
        Script::<TestError>::default(),
        Duration::from_secs(42),
        token,
    );
    let (logs, _guard) = capture();
    let outcome = call_tool(
        &mut client,
        &request(),
        None::<&RecordingHost>,
        swallowing(),
    )
    .await;
    assert!(matches!(outcome, Err(SessionError::Cancelled)));
    let cancelled = logs.matching("MCP client tool call timed out after");
    assert_eq!(cancelled.len(), 1);
    assert!(cancelled[0].contains("42"));
    assert!(logs.matching("traceback").is_empty());
    assert_eq!(logs.count(Level::ERROR), 0);
}

#[tokio::test(start_paused = true)]
async fn a_cleanup_timeout_propagates_as_cancellation() {
    let script = hanging::<TestError>(Vec::new(), CallOutcome::Echo);
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let outcome = call_tool(
        &mut client,
        &request(),
        None::<&RecordingHost>,
        swallowing(),
    )
    .await;
    assert!(matches!(outcome, Err(SessionError::CleanupTimedOut)));
    assert_eq!(
        logs.matching("MCP client tool call timed out after").len(),
        1
    );
    assert!(logs.matching("traceback").is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_cleanup_timeout_cancels_a_successful_listing() {
    let script = hanging::<TestError>(
        vec![super::tests::fixture::page(&["first"], None)],
        CallOutcome::Echo,
    );
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let outcome = list_tools(&mut client, Duration::from_secs(30), false).await;
    assert!(matches!(outcome, Err(SessionError::CleanupTimedOut)));
    assert_eq!(
        logs.matching("MCP client list_tools was cancelled").len(),
        1
    );
}
