use betterlite_mcp::client::tools::result::{CallToolResult, InputRequiredResult, ToolCallResult};
use betterlite_mcp::tools::schemas::McpTool;
use betterlite_mcp::wire_compat::result_conversion::{
    complete_call_tool_result, to_call_tool_result,
};
use betterlite_mcp::wire_compat::tool_outcome::{
    JsonResult, TextResult, ToolFailure, ToolOutcome, WireCompat, parse_http_body,
};
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperResult;

use super::failure;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtocolFields {
    pub(super) protocol_version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyFields {
    body: String,
}

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
#[serde(deny_unknown_fields)]
pub(crate) struct GatewayToolFields {
    pub(super) tool: McpTool,
    pub(super) name: String,
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

pub(crate) fn run_parse_body(fields: &BodyFields) -> HelperResult {
    match parse_http_body(&fields.body) {
        ToolOutcome::Text(parsed) => Ok(json!({ "kind": "text", "text": parsed.text })),
        ToolOutcome::Json(parsed) => Ok(json!({
            "kind": "json",
            "value": parsed.value,
            "original_text": parsed.original_text,
        })),
        ToolOutcome::Complete(_) | ToolOutcome::InputRequired(_) | ToolOutcome::Failure(_) => {
            Err(failure(
                "UnexpectedOutcome",
                "an HTTP body parses to text or JSON".to_owned(),
            ))
        }
    }
}

pub(crate) enum Conversion {
    Wire,
    Complete,
}

pub(crate) fn run_convert(fields: &ConvertFields, conversion: &Conversion) -> Value {
    match conversion {
        Conversion::Wire => match to_call_tool_result(fields.outcome(), fields.compat) {
            ToolCallResult::Complete(result) => json!({ "complete": result }),
            ToolCallResult::InputRequired(result) => json!({ "input_required": result }),
        },
        Conversion::Complete => {
            json!({ "complete": complete_call_tool_result(fields.outcome(), fields.compat) })
        }
    }
}
