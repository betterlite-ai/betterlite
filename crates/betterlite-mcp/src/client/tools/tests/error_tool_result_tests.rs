#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_prompts::arguments::json::Object;
use tokio_util::sync::CancellationToken;

use super::tests::fixture::{
    CallOutcome, ErrorScript, RecordingHost, TestError, build_client, calling,
};
use crate::client::sessions::error::{McpFailure, SessionError};
use crate::client::tools::calling::{CallToolOptions, call_tool, error_tool_result};
use crate::client::tools::result::{CallToolResult, ContentBlock, TextContent, ToolCallResult};
use crate::tools::tool_calls::CallRequest;

fn error_result(error: &SessionError<TestError>) -> ToolCallResult {
    error_tool_result(error)
}

fn text_of(result: &ToolCallResult) -> &str {
    let ToolCallResult::Complete(complete) = result else {
        panic!("expected a complete result, got {result:?}");
    };
    match complete.content.as_slice() {
        [ContentBlock::Text(TextContent { text, .. })] => text,
        other => panic!("expected one text block, got {other:?}"),
    }
}

#[test]
fn builds_a_text_result_naming_the_error_type_and_message() {
    let error = SessionError::Runtime(Arc::new(TestError::Boom("tool crashed")));
    let result = error_result(&error);
    assert_eq!(
        result,
        ToolCallResult::Complete(CallToolResult {
            content: vec![ContentBlock::Text(TextContent {
                text: "TestError: upstream failure: tool crashed".to_owned(),
                extra: Object::new(),
            })],
            is_error: true,
            structured_content: None,
            meta: None,
        })
    );
}

#[test]
fn carries_a_protocol_failure_message() {
    let error = SessionError::<TestError>::Protocol(McpFailure {
        code: -32_022,
        message: "unsupported version".to_owned(),
        context: None,
    });
    assert_eq!(
        text_of(&error_result(&error)),
        "MCPError: unsupported version"
    );
}

#[tokio::test]
async fn serves_the_obo_retry_call_site() {
    let script = calling::<TestError>(
        CallOutcome::Fail(ErrorScript::Runtime(TestError::Boom("upstream 401"))),
        Vec::new(),
    );
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let options = CallToolOptions {
        raise_on_error: true,
        allow_input_required: false,
    };
    let request = CallRequest {
        name: "echo".to_owned(),
        arguments: None,
    };
    let Err(error) = call_tool(&mut client, &request, None::<&RecordingHost>, options).await else {
        panic!("the caller owns the exception");
    };
    let result = error_result(&error);
    let ToolCallResult::Complete(complete) = &result else {
        panic!("expected an error result, got {result:?}");
    };
    assert!(complete.is_error);
    assert!(text_of(&result).contains("upstream 401"));
}

#[tokio::test]
async fn serves_the_passthrough_call_site() {
    let script = calling::<TestError>(
        CallOutcome::Fail(ErrorScript::Runtime(TestError::Boom("upstream 500"))),
        Vec::new(),
    );
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let options = CallToolOptions {
        raise_on_error: true,
        allow_input_required: false,
    };
    let request = CallRequest {
        name: "echo".to_owned(),
        arguments: None,
    };
    let Err(error) = call_tool(&mut client, &request, None::<&RecordingHost>, options).await else {
        panic!("the caller owns the exception");
    };
    let result = error_result(&error);
    let ToolCallResult::Complete(complete) = &result else {
        panic!("expected an error result, got {result:?}");
    };
    assert!(complete.is_error);
    assert!(text_of(&result).contains("upstream 500"));
}
