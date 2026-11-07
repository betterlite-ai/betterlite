#![cfg(test)]

use std::time::Duration;

use betterlite_prompts::arguments::json::{Json, Object};
use tokio_util::sync::CancellationToken;
use tracing::Level;

use super::tests::fixture::{
    BrokenResource, CallOutcome, ErrorScript, ProgressEvent, RecordingHost, Script, TestError,
    build_client, calling, capture, protocol_failure, text_result,
};
use crate::client::sessions::error::SessionError;
use crate::client::tools::calling::{CallToolOptions, call_tool};
use crate::client::tools::result::{
    CallToolResult, ContentBlock, InputRequiredResult, TextContent, ToolCallResult,
};
use crate::tools::tool_calls::CallRequest;

fn swallow() -> CallToolOptions {
    CallToolOptions {
        raise_on_error: false,
        allow_input_required: false,
    }
}

fn request() -> CallRequest {
    CallRequest {
        name: "echo".to_owned(),
        arguments: Some(Object::from([(
            "city".to_owned(),
            Json::Text("Paris".to_owned()),
        )])),
    }
}

fn fail(error: TestError) -> CallOutcome<TestError> {
    CallOutcome::Fail(ErrorScript::Runtime(error))
}

fn text_of(result: &CallToolResult) -> &str {
    match result.content.as_slice() {
        [ContentBlock::Text(TextContent { text, .. })] => text,
        other => panic!("expected one text block, got {other:?}"),
    }
}

fn complete(result: &ToolCallResult) -> &CallToolResult {
    match result {
        ToolCallResult::Complete(value) => value,
        ToolCallResult::InputRequired(other) => panic!("expected a complete result, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_the_session_result_on_success() {
    let script = calling::<TestError>(
        CallOutcome::Complete(text_result("tool output")),
        Vec::new(),
    );
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let result = call_tool(&mut client, &request(), None::<&RecordingHost>, swallow())
        .await
        .unwrap();
    assert_eq!(result, ToolCallResult::Complete(text_result("tool output")));
    assert_eq!(logs.matching("MCP client calling tool 'echo'").len(), 1);
    assert_eq!(logs.matching("completed successfully").len(), 1);
}

#[tokio::test]
async fn forwards_the_request_to_the_session() {
    let mut client = build_client(
        Script::<TestError>::default(),
        Duration::from_secs(60),
        CancellationToken::new(),
    );
    let result = call_tool(&mut client, &request(), None::<&RecordingHost>, swallow())
        .await
        .unwrap();
    let text = text_of(complete(&result));
    assert!(text.contains("echo"));
    assert!(text.contains("Paris"));
}

#[tokio::test]
async fn returns_an_error_result_when_the_server_fails() {
    let script = calling::<TestError>(fail(TestError::Boom("tool crashed")), Vec::new());
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let result = call_tool(&mut client, &request(), None::<&RecordingHost>, swallow())
        .await
        .unwrap();
    let complete = complete(&result);
    assert!(complete.is_error);
    assert_eq!(
        text_of(complete),
        "TestError: upstream failure: tool crashed"
    );
    assert_eq!(logs.matching("MCP client call_tool failed").len(), 1);
    assert_eq!(logs.count(Level::ERROR), 1);
}

#[tokio::test]
async fn re_raises_when_the_caller_owns_the_error() {
    let script = calling::<TestError>(fail(TestError::Boom("tool crashed")), Vec::new());
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let options = CallToolOptions {
        raise_on_error: true,
        allow_input_required: false,
    };
    let outcome = call_tool(&mut client, &request(), None::<&RecordingHost>, options).await;
    match outcome {
        Err(SessionError::Runtime(error)) => assert_eq!(*error, TestError::Boom("tool crashed")),
        other => panic!("expected the upstream error, got {other:?}"),
    }
    assert_eq!(logs.count(Level::ERROR), 0);
    assert_eq!(
        logs.events()
            .into_iter()
            .filter(|(level, message)| *level == Level::DEBUG
                && message.contains("MCP client call_tool failed"))
            .count(),
        1
    );
}

#[tokio::test]
async fn accepts_an_input_required_result_when_allowed() {
    let expected = InputRequiredResult {
        input_requests: Some(Object::from([(
            "name".to_owned(),
            Json::Object(Object::new()),
        )])),
        request_state: Some("state-1".to_owned()),
        result_type: "input_required".to_owned(),
        meta: None,
    };
    let script = calling::<TestError>(CallOutcome::InputRequired(expected.clone()), Vec::new());
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let options = CallToolOptions {
        raise_on_error: false,
        allow_input_required: true,
    };
    let result = call_tool(&mut client, &request(), None::<&RecordingHost>, options)
        .await
        .unwrap();
    assert_eq!(result, ToolCallResult::InputRequired(expected));
}

#[tokio::test]
async fn rejects_an_input_required_result_when_not_allowed() {
    let expected = InputRequiredResult {
        input_requests: None,
        request_state: Some("state-1".to_owned()),
        result_type: "input_required".to_owned(),
        meta: None,
    };
    let script = calling::<TestError>(CallOutcome::InputRequired(expected), Vec::new());
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let result = call_tool(&mut client, &request(), None::<&RecordingHost>, swallow())
        .await
        .unwrap();
    let complete = complete(&result);
    assert!(complete.is_error);
    assert_eq!(
        text_of(complete),
        "TestError: the server returned an input_required result"
    );
}

#[tokio::test]
async fn forwards_progress_to_the_host() {
    let script = calling::<TestError>(
        CallOutcome::Echo,
        vec![
            ProgressEvent {
                progress: 1.0_f64,
                total: Some(2.0_f64),
                message: Some("half".to_owned()),
            },
            ProgressEvent {
                progress: 2.0_f64,
                total: Some(2.0_f64),
                message: None,
            },
        ],
    );
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let host = RecordingHost::default();
    let result = call_tool(&mut client, &request(), Some(&host), swallow())
        .await
        .unwrap();
    assert!(matches!(result, ToolCallResult::Complete(_)));
    assert_eq!(
        host.seen(),
        [(1.0_f64, Some(2.0_f64)), (2.0_f64, Some(2.0_f64))]
    );
    assert_eq!(logs.matching("progress:").len(), 2);
}

#[tokio::test]
async fn keeps_the_result_when_the_host_rejects_progress() {
    let script = calling::<TestError>(
        CallOutcome::Echo,
        vec![ProgressEvent {
            progress: 1.0_f64,
            total: Some(2.0_f64),
            message: None,
        }],
    );
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let host = RecordingHost::failing_on(1);
    let result = call_tool(&mut client, &request(), Some(&host), swallow())
        .await
        .unwrap();
    assert!(!complete(&result).is_error);
    assert_eq!(logs.matching("Failed to forward to Host").len(), 1);
    assert_eq!(logs.count(Level::WARN), 1);
}

#[tokio::test]
async fn logs_a_broken_connection_diagnostic() {
    let script = calling::<BrokenResource>(
        CallOutcome::Fail(ErrorScript::Runtime(BrokenResource)),
        Vec::new(),
    );
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let outcome = call_tool(&mut client, &request(), None::<&RecordingHost>, swallow()).await;
    assert!(matches!(outcome, Ok(ToolCallResult::Complete(_))));
    assert_eq!(logs.count(Level::ERROR), 2);
    assert_eq!(
        logs.matching("MCP client detected broken connection/stream -")
            .len(),
        1
    );
}

#[tokio::test]
async fn checks_only_the_error_name_for_a_broken_connection() {
    let script = calling::<TestError>(CallOutcome::Fail(protocol_failure()), Vec::new());
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let result = call_tool(&mut client, &request(), None::<&RecordingHost>, swallow())
        .await
        .unwrap();
    assert!(complete(&result).is_error);
    assert_eq!(logs.matching("MCP client call_tool failed").len(), 1);
    assert!(logs.matching("detected broken connection").is_empty());
}

#[tokio::test]
async fn logs_the_traceback_only_at_debug_level() {
    let script = calling::<TestError>(fail(TestError::Boom("tool crashed")), Vec::new());
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    drop(call_tool(&mut client, &request(), None::<&RecordingHost>, swallow()).await);
    let tracebacks = logs.matching("MCP client tool call traceback");
    assert_eq!(tracebacks.len(), 1);
    assert!(tracebacks[0].contains("tool crashed"));
    assert!(
        logs.events()
            .into_iter()
            .filter(|(level, _)| *level == Level::ERROR)
            .all(|(_, message)| !message.contains("traceback")
                && !message.contains("tool crashed"))
    );
}

#[test]
fn keeps_unmodeled_content_and_result_fields() {
    let encoded = r#"{"content":[{"type":"text","text":"hi","annotations":{"audience":["user"]}}],"isError":true,"structuredContent":{"a":1},"_meta":{"b":2}}"#;
    let result: CallToolResult = serde_json::from_str(encoded).unwrap();
    assert!(result.is_error);
    let structured = Object::from([(
        "a".to_owned(),
        Json::Number(serde_json::Number::from(1_u64)),
    )]);
    assert_eq!(result.structured_content, Some(Json::Object(structured)));
    assert_eq!(
        result.meta,
        Some(Object::from([(
            "b".to_owned(),
            Json::Number(serde_json::Number::from(2_u64))
        )]))
    );
    let [ContentBlock::Text(text)] = result.content.as_slice() else {
        panic!("expected one text block");
    };
    assert_eq!(text.text, "hi");
    let audience = Json::List(vec![Json::Text("user".to_owned())]);
    let annotations = Json::Object(Object::from([("audience".to_owned(), audience)]));
    assert_eq!(text.extra.get("annotations"), Some(&annotations));
}
