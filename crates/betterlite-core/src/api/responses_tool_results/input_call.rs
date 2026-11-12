use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::ToolResultsError;
use super::records::{FunctionCallMessage, FunctionChunk, ToolCallChunk, text_field};
use crate::api::responses_codex_tools::custom_tools::serialize_tool_call_arguments;

const REDACTED_BY_LITELLM: &str = "redacted-by-litellm";
const REDACTED_ARGUMENTS: &str = "{}";

// a custom tool call keeps its raw string in `input`; chat tool calls carry JSON arguments
fn input_arguments(
    item: &IndexMap<String, Json>,
    custom: bool,
) -> Result<String, ToolResultsError> {
    let raw_input = if custom {
        text_field(item, "input", "")?
    } else {
        ""
    };
    if raw_input.is_empty() {
        return Ok(String::new());
    }
    let wrapped = IndexMap::from([("content".to_owned(), Json::Text(raw_input.to_owned()))]);
    Ok(dumps(&Json::Object(wrapped)))
}

pub fn input_function_call(
    item: &IndexMap<String, Json>,
) -> Result<FunctionCallMessage, ToolResultsError> {
    let custom = text_field(item, "type", "")? == "custom_tool_call";
    let raw_arguments = item.get("arguments").unwrap_or(&Json::Null);
    let blank = match raw_arguments {
        Json::Null => true,
        Json::Text(text) => text.is_empty(),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    };
    let arguments = if matches!(raw_arguments, Json::Text(text) if text == REDACTED_BY_LITELLM) {
        REDACTED_ARGUMENTS.to_owned()
    } else if blank {
        input_arguments(item, custom)?
    } else {
        serialize_tool_call_arguments(raw_arguments, "")
    };
    let name = text_field(item, "name", "")?;
    let namespace = text_field(item, "namespace", "")?;
    let qualified = if namespace.is_empty() || custom {
        name.to_owned()
    } else {
        format!("{namespace}__{name}")
    };
    let call = ToolCallChunk {
        id: text_field(item, "call_id", text_field(item, "id", "")?)?.to_owned(),
        kind: "function".to_owned(),
        function: FunctionChunk {
            name: qualified,
            arguments,
            provider_specific_fields: None,
        },
        index: 0,
        provider_specific_fields: None,
    };
    Ok(FunctionCallMessage {
        role: "assistant",
        content: None,
        tool_calls: vec![call],
    })
}
