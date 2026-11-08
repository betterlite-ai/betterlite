use crate::arguments::json::{Json, Object};
use crate::cohere::error::CohereError;
use crate::cohere::types::{CohereCall, CohereTool, CohereToolCall};
use crate::gemini_tool_invokes::limits::MAX_TOOL_CALLS;
use crate::message_content::Message;
use serde::Deserialize;

const FUNCTION_TYPE: &str = "function";
const TOOL_CALL_ID: &str = "tool_call_id";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MessageToolCalls {
    tool_calls: Option<Vec<CohereTool>>,
}

pub fn collect_tool_calls(messages: &[Message]) -> Result<Vec<CohereTool>, CohereError> {
    let mut collected = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let tools = message_tool_calls(message, index)?;
        if collected.len().saturating_add(tools.len()) > MAX_TOOL_CALLS {
            return Err(CohereError::TooManyToolCalls {
                limit: MAX_TOOL_CALLS,
            });
        }
        collected.extend(tools);
    }
    Ok(collected)
}

pub(crate) fn message_tool_calls(
    message: &Message,
    index: usize,
) -> Result<Vec<CohereTool>, CohereError> {
    let fields = serde_json::to_value(&message.provider_specific_fields)
        .map_err(|source| CohereError::FieldsEncode { index, source })?;
    let decoded: MessageToolCalls = serde_path_to_error::deserialize(fields)
        .map_err(|source| CohereError::Fields { index, source })?;
    Ok(decoded.tool_calls.unwrap_or_default())
}

pub fn convert_to_cohere_tool_invoke(
    tool_calls: &[CohereTool],
) -> Result<Vec<CohereToolCall>, CohereError> {
    if tool_calls.len() > MAX_TOOL_CALLS {
        return Err(CohereError::TooManyToolCalls {
            limit: MAX_TOOL_CALLS,
        });
    }
    let mut invokes = Vec::new();
    for (index, tool) in tool_calls.iter().enumerate() {
        if tool.kind.as_deref() != Some(FUNCTION_TYPE) {
            continue;
        }
        invokes.push(invoke(tool, index)?);
    }
    Ok(invokes)
}

fn invoke(tool: &CohereTool, index: usize) -> Result<CohereToolCall, CohereError> {
    let Some(function) = tool.function.as_ref() else {
        return Err(CohereError::MissingFunction { index });
    };
    let Some(arguments) = function.arguments.as_deref() else {
        return Err(CohereError::MissingArguments { index });
    };
    let parameters = serde_json::from_str::<Json>(arguments)
        .map_err(|source| CohereError::Arguments { index, source })?;
    Ok(CohereToolCall {
        name: function.name.clone(),
        parameters,
    })
}

pub(crate) fn recovered_call(
    message: &Message,
    tool_calls: &[CohereTool],
) -> Result<CohereCall, CohereError> {
    let mut name = String::new();
    let mut parameters = Json::Object(Object::new());
    let message_id = message
        .provider_specific_fields
        .get(TOOL_CALL_ID)
        .and_then(Json::as_str)
        .filter(|id| !id.is_empty());
    let Some(call_id) = message_id else {
        return Ok(CohereCall { name, parameters });
    };
    for (index, tool) in tool_calls.iter().enumerate() {
        let Some(previous_id) = tool.id.as_deref().filter(|id| !id.is_empty()) else {
            continue;
        };
        if previous_id != call_id {
            continue;
        }
        name = tool
            .function
            .as_ref()
            .and_then(|function| function.name.clone())
            .unwrap_or_default();
        if let Some(parsed) = arguments_of(tool, index)? {
            parameters = parsed;
        }
    }
    Ok(CohereCall { name, parameters })
}

fn arguments_of(tool: &CohereTool, index: usize) -> Result<Option<Json>, CohereError> {
    let arguments = tool
        .function
        .as_ref()
        .and_then(|function| function.arguments.as_deref());
    let Some(raw) = arguments.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    serde_json::from_str::<Json>(raw)
        .map(Some)
        .map_err(|source| CohereError::Arguments { index, source })
}
