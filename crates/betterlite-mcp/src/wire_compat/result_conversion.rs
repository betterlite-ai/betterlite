use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::client::tools::result::{
    CallToolResult, ContentBlock, TextContent, ToolCallResult, error_text_result,
};
use crate::tools::schemas::McpTool;

use super::tool_outcome::{ToolOutcome, WireCompat};

pub const INPUT_REQUIRED_UNSUPPORTED: &str = "Error: upstream tool returned an input_required interim result, which this MCP protocol revision cannot carry";

#[must_use]
pub fn to_call_tool_result(outcome: ToolOutcome, compat: WireCompat) -> ToolCallResult {
    match outcome {
        ToolOutcome::Text(result) => ToolCallResult::Complete(text_result(result.text, None)),
        ToolOutcome::Json(result) => {
            let keep_structured =
                compat == WireCompat::Modern || matches!(&result.value, Json::Object(_));
            let structured = keep_structured.then_some(result.value);
            ToolCallResult::Complete(text_result(result.original_text, structured))
        }
        ToolOutcome::Complete(result) => ToolCallResult::Complete(match compat {
            WireCompat::Modern => result,
            WireCompat::Legacy => downgrade_structured_content(result),
        }),
        ToolOutcome::InputRequired(result) => match compat {
            WireCompat::Modern => ToolCallResult::InputRequired(result),
            WireCompat::Legacy => ToolCallResult::Complete(input_required_unsupported()),
        },
        ToolOutcome::Failure(failure) => error_text_result(&failure.error_type, &failure.message),
    }
}

#[must_use]
pub fn complete_call_tool_result(outcome: ToolOutcome, compat: WireCompat) -> CallToolResult {
    match to_call_tool_result(outcome, compat) {
        ToolCallResult::Complete(result) => result,
        ToolCallResult::InputRequired(_) => input_required_unsupported(),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    pub src: String,
    pub mime_type: Option<String>,
    pub sizes: Option<Vec<String>>,
    pub theme: Option<Theme>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskSupport {
    Forbidden,
    Optional,
    Required,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    pub task_support: Option<TaskSupport>,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotations {
    pub title: Option<String>,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

// the Tool keys the SDK declares: an undeclared key or a null member never reaches the wire
#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Object>,
    pub title: Option<String>,
    pub execution: Option<Execution>,
    pub output_schema: Option<Object>,
    pub icons: Option<Vec<Icon>>,
    pub annotations: Option<Annotations>,
    #[serde(rename = "_meta")]
    pub meta: Option<Object>,
}

#[derive(Debug, Error)]
#[error("invalid tool: {0}")]
pub struct InvalidTool(#[from] serde_json::Error);

pub fn to_gateway_tool(tool: &McpTool, name: &str) -> Result<GatewayTool, InvalidTool> {
    let mut gateway: GatewayTool = serde_json::to_value(tool).and_then(serde_json::from_value)?;
    name.clone_into(&mut gateway.name);
    Ok(gateway)
}

fn text_result(text: String, structured_content: Option<Json>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text,
            extra: Object::new(),
        })],
        is_error: false,
        structured_content,
        meta: None,
    }
}

fn input_required_unsupported() -> CallToolResult {
    CallToolResult {
        is_error: true,
        ..text_result(INPUT_REQUIRED_UNSUPPORTED.to_owned(), None)
    }
}

fn downgrade_structured_content(result: CallToolResult) -> CallToolResult {
    let Some(structured) = result
        .structured_content
        .as_ref()
        .filter(|structured| !matches!(structured, Json::Object(_)))
    else {
        return result;
    };
    let mut content = result.content;
    content.push(ContentBlock::Text(TextContent {
        text: serde_json::to_string(structured).unwrap_or_default(),
        extra: Object::new(),
    }));
    CallToolResult {
        content,
        structured_content: None,
        ..result
    }
}

mod tests;
