use std::time::Duration;

use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::error::{McpFailure, SessionError};
use betterlite_mcp::client::tools::calling::error_tool_result;
use betterlite_mcp::client::tools::result::ToolCallResult;
use betterlite_mcp::tools::pagination::{LoadedTool, Page, ToolError};
use betterlite_mcp::tools::schemas::McpTool;
use betterlite_prompts::arguments::json::Object;

use super::{closed_payload, finish};
use crate::mcp::convert::{NativeMcpErrorPayload, session_payload, tool_failure};

fn failure(code: i32) -> SessionError<SdkError> {
    SessionError::Protocol(McpFailure {
        code,
        message: "denied".to_owned(),
        context: None,
    })
}

fn tool() -> McpTool {
    McpTool {
        name: "search".to_owned(),
        description: None,
        input_schema: None,
        extra: Object::new(),
    }
}

#[test]
fn session_page_answer_keeps_the_cursor_name() {
    let page = Page {
        tools: vec![tool()],
        next_cursor: Some("page2".to_owned()),
    };
    let text = finish(Ok(page), session_payload).unwrap();
    assert!(text.contains("\"nextCursor\":\"page2\""));
}

#[test]
fn session_page_error_keeps_its_code() {
    let payload = finish::<Page, _>(Err(failure(-32603)), session_payload).unwrap_err();
    assert_eq!(payload.code, Some(-32603));
    assert_eq!(payload.message, "denied");
}

#[test]
fn session_call_answer_carries_the_result() {
    let text = finish(Ok(error_tool_result("McpError", "boom")), session_payload).unwrap();
    assert!(text.contains("\"isError\":true"));
}

#[test]
fn session_call_error_is_a_payload() {
    let outcome: Result<ToolCallResult, _> = Err(failure(-32000));
    let payload = finish(outcome, session_payload).unwrap_err();
    assert_eq!(payload.code, Some(-32000));
}

#[test]
fn session_tools_answer_lists_both_shapes() {
    let tools = vec![LoadedTool::Mcp(tool())];
    let text = finish(Ok(tools), tool_failure).unwrap();
    assert!(text.contains("\"search\""));
}

#[test]
fn session_tools_session_error_is_a_payload() {
    let payload =
        finish::<Vec<LoadedTool>, _>(Err(ToolError::Session(failure(-32602))), tool_failure)
            .unwrap_err();
    assert_eq!(payload.code, Some(-32602));
}

#[tokio::test]
async fn session_tools_timeout_is_a_timeout_payload() {
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let payload =
        finish::<Vec<LoadedTool>, _>(Err(ToolError::Timeout(elapsed)), tool_failure).unwrap_err();
    assert!(payload.message.contains("timed out"));
}

#[test]
fn session_output_answer_carries_the_result() {
    let text = finish(Ok(error_tool_result("McpError", "boom")), tool_failure).unwrap();
    assert!(text.contains("boom"));
}

#[test]
fn session_output_error_is_a_payload() {
    let outcome: Result<ToolCallResult, _> = Err(ToolError::Session(failure(-32001)));
    let payload = finish(outcome, tool_failure).unwrap_err();
    assert_eq!(payload.code, Some(-32001));
}

#[test]
fn session_closed_payload_is_a_runtime_error() {
    let payload: NativeMcpErrorPayload = closed_payload();
    let text = serde_json::to_string(&payload).unwrap();
    assert!(text.contains("\"type\":\"RuntimeError\""));
    assert!(text.contains("closed"));
}
