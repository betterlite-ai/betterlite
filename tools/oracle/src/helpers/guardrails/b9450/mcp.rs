use std::sync::Arc;

use betterlite_guardrails::cisco_mcp_tool_hook::CiscoMcpToolHook;
use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::mcp_tool_hook::types::{
    McpHiddenParams, McpPostCallResponse, McpToolHookCall, McpToolResult,
};
use betterlite_oracle_helper_api::helper::HelperResult;
use chrono::DateTime;
use serde_json::{Value, json};

use super::McpArgs;
use super::projection;
use super::support::{Harness, fail, selection};

const TIMESTAMP: &str = "2024-01-01T00:00:00+00:00";

pub(super) fn run(args: &McpArgs) -> HelperResult {
    let harness = Harness::start(&args.answer)?;
    let hook = CiscoMcpToolHook {
        guardrail: Arc::new(harness.guardrail(&args.settings)?),
        selection: selection(&args.settings)?,
    };
    let mut response = McpPostCallResponse {
        mcp_tool_call_response: args.tool_response.clone(),
        hidden_params: McpHiddenParams {
            response_cost: None,
        },
    };
    // The proxy builds original_response from the same tool result the response copy holds.
    let mut original = McpToolResult {
        content: args.tool_response.clone(),
        structured_content: None,
        is_error: false,
    };
    let timestamp = DateTime::parse_from_rfc3339(TIMESTAMP)
        .map_err(|error| fail("InvalidInput", error.to_string()))?;
    let returned = harness
        .runtime
        .block_on(hook.async_post_mcp_tool_call_hook(McpToolHookCall {
            kwargs: &args.kwargs,
            response: &mut response,
            original: &mut original,
            start_time: timestamp,
            end_time: timestamp,
        }))
        .map_err(|error| fail("LoggerError", error.to_string()))?;
    let (path, inspected) = harness.recorded()?;
    Ok(report(
        returned.as_ref(),
        &response,
        &original,
        path,
        inspected,
    ))
}

fn report(
    returned: Option<&McpPostCallResponse>,
    response: &McpPostCallResponse,
    original: &McpToolResult,
    path: Option<String>,
    inspected: Option<Value>,
) -> Value {
    let returned_items = returned.map_or(Value::Null, |value| {
        Value::Array(
            value
                .mcp_tool_call_response
                .iter()
                .map(projection::content_item)
                .collect(),
        )
    });
    json!({
        "returned": returned_items,
        "response": Value::Array(response.mcp_tool_call_response.iter().map(projection::content_item).collect()),
        "original": Value::Array(original.content.iter().map(projection::content_item).collect()),
        "path": path.map_or(Value::Null, Value::String),
        "request": inspected.unwrap_or(Value::Null),
    })
}
