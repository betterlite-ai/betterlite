use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::client::tools::result::{
    CallToolResult, ContentBlock, TextContent, ToolCallResult, error_text_result,
};
use crate::tools::schemas::McpTool;

use super::tool_outcome::{ToolOutcome, WireCompat};

pub const INPUT_REQUIRED_UNSUPPORTED: &str = "Error: upstream tool returned an input_required interim result, which this MCP protocol revision cannot carry";

#[must_use]
pub fn to_call_tool_result(outcome: ToolOutcome, compat: WireCompat) -> ToolCallResult {
    match outcome {
        ToolOutcome::Text(result) => ToolCallResult::Complete(CallToolResult {
            content: vec![ContentBlock::Text(TextContent {
                text: result.text,
                extra: Object::new(),
            })],
            is_error: false,
            structured_content: None,
            meta: None,
        }),
        ToolOutcome::Json(result) => {
            let keep_structured =
                compat == WireCompat::Modern || matches!(&result.value, Json::Object(_));
            ToolCallResult::Complete(CallToolResult {
                content: vec![ContentBlock::Text(TextContent {
                    text: result.original_text,
                    extra: Object::new(),
                })],
                is_error: false,
                structured_content: keep_structured.then_some(result.value),
                meta: None,
            })
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GatewayTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Object>,
    #[serde(flatten)]
    pub extra: Object,
}

// LiteLLM dumps the tool with exclude_none, so a null optional never reaches the wire
#[must_use]
pub fn to_gateway_tool(tool: &McpTool, name: &str) -> GatewayTool {
    GatewayTool {
        name: name.to_owned(),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
        extra: tool
            .extra
            .iter()
            .filter(|(_, value)| !matches!(value, Json::Null))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    }
}

fn input_required_unsupported() -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: INPUT_REQUIRED_UNSUPPORTED.to_owned(),
            extra: Object::new(),
        })],
        is_error: true,
        structured_content: None,
        meta: None,
    }
}

fn downgrade_structured_content(result: CallToolResult) -> CallToolResult {
    let Some(structured) = result.structured_content.as_ref() else {
        return result;
    };
    if matches!(structured, Json::Object(_)) {
        return result;
    }
    let mut content = result.content;
    content.push(ContentBlock::Text(TextContent {
        text: serde_json::to_string(structured).unwrap_or_default(),
        extra: Object::new(),
    }));
    CallToolResult {
        content,
        is_error: result.is_error,
        structured_content: None,
        meta: result.meta,
    }
}

mod tests;
