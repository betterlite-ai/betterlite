use betterlite_mcp::client::tools::result::{CallToolResult, InputRequiredResult, ToolCallResult};
use betterlite_mcp::tools::schemas::McpTool;
use betterlite_mcp::wire_compat::result_conversion::{
    complete_call_tool_result, to_call_tool_result, to_gateway_tool,
};
use betterlite_mcp::wire_compat::tool_outcome::{
    JsonResult, TextResult, ToolFailure, ToolOutcome, WireCompat, parse_http_body, wire_compat_for,
};
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::HelperResult;

use super::failure;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum OutcomeFields {
    Text { text: String },
    Json { value: Json, original_text: String },
    CallToolResult { result: CallToolResult },
    InputRequired { result: InputRequiredResult },
    Exception { type_name: String, message: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConvertFields {
    outcome: OutcomeFields,
    compat: WireCompat,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ConversionInput {
    WireCompatFor { protocol_version: String },
    ParseHttpBody { body: String },
    ToCallToolResult(ConvertFields),
    CompleteCallToolResult(ConvertFields),
    ToGatewayTool { tool: McpTool, name: String },
}

pub(crate) fn run(input: &ConversionInput) -> HelperResult {
    match input {
        ConversionInput::WireCompatFor { protocol_version } => {
            Ok(json!(wire_compat_for(protocol_version)))
        }
        ConversionInput::ParseHttpBody { body } => match parse_http_body(body) {
            ToolOutcome::Text(parsed) => Ok(json!({ "kind": "text", "text": parsed.text })),
            ToolOutcome::Json(parsed) => Ok(json!({
                "kind": "json",
                "value": parsed.value,
                "original_text": parsed.original_text,
            })),
            ToolOutcome::Complete(_) | ToolOutcome::InputRequired(_) | ToolOutcome::Failure(_) => {
                Err(failure("UnexpectedOutcome", String::new()))
            }
        },
        ConversionInput::ToCallToolResult(fields) => {
            Ok(match to_call_tool_result(fields.outcome(), fields.compat) {
                ToolCallResult::Complete(result) => json!({ "complete": result }),
                ToolCallResult::InputRequired(result) => json!({ "input_required": result }),
            })
        }
        ConversionInput::CompleteCallToolResult(fields) => Ok(json!({
            "complete": complete_call_tool_result(fields.outcome(), fields.compat)
        })),
        ConversionInput::ToGatewayTool { tool, name } => to_gateway_tool(tool, name)
            .map(|gateway| json!(gateway))
            .map_err(|error| failure("InvalidTool", error.to_string())),
    }
}

impl ConvertFields {
    fn outcome(&self) -> ToolOutcome {
        match self.outcome.clone() {
            OutcomeFields::Text { text } => ToolOutcome::Text(TextResult { text }),
            OutcomeFields::Json {
                value,
                original_text,
            } => ToolOutcome::Json(JsonResult {
                value,
                original_text,
            }),
            OutcomeFields::CallToolResult { result } => ToolOutcome::Complete(result),
            OutcomeFields::InputRequired { result } => ToolOutcome::InputRequired(result),
            OutcomeFields::Exception { type_name, message } => ToolOutcome::Failure(ToolFailure {
                error_type: type_name,
                message,
            }),
        }
    }
}
