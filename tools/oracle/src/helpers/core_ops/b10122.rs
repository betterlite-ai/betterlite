use std::collections::BTreeMap;

use betterlite_core::api::responses_mcp_tool_selection::{
    McpToolCall, extract_tool_call_details, should_auto_execute_tools,
};
// ast-grep-ignore: no-long-names
use betterlite_core::api::responses_mcp_tool_selection::extract_tool_calls_from_response as extract_calls;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    ShouldAutoExecuteTools { references: Vec<Json> },
    ExtractToolCallsFromResponse { response: Box<ResponsesApiResponse> },
    ExtractToolCallDetails { tool_call: ToolCallInput },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ToolCallInput {
    Output { item: Box<ResponseOutputItem> },
    Mapping { fields: BTreeMap<String, Json> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(answer)
            .collect::<Result<Vec<Value>, _>>()?;
        Ok(Value::Array(answers))
    }
}

fn answer(call: &Call) -> HelperResult {
    match call {
        Call::ShouldAutoExecuteTools { references } => {
            Ok(Value::Bool(should_auto_execute_tools(references)))
        }
        Call::ExtractToolCallsFromResponse { response } => {
            let views = extract_calls(response)
                .iter()
                .map(details_value)
                .collect::<Result<Vec<Value>, _>>()?;
            Ok(Value::Array(views))
        }
        Call::ExtractToolCallDetails { tool_call } => match tool_call {
            ToolCallInput::Output { item } => details_value(&McpToolCall::Output(item.clone())),
            ToolCallInput::Mapping { fields } => {
                details_value(&McpToolCall::Fields(fields.clone()))
            }
        },
    }
}

fn details_value(call: &McpToolCall) -> HelperResult {
    extract_tool_call_details(call)
        .or_value_error()
        .map(|details| json!([details.name, details.arguments, details.call_id]))
}
