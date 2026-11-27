use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};

use crate::api::responses_mcp_events::function_call::function_call_id;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToolSelectionError {
    #[error("{field} must be a string")]
    NotAString { field: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpToolCall {
    Output(Box<ResponseOutputItem>),
    Fields(BTreeMap<String, Json>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolCallDetails {
    pub name: Option<String>,
    pub arguments: Option<Json>,
    pub call_id: Option<String>,
}

#[must_use]
pub fn should_auto_execute_tools(references: &[Json]) -> bool {
    !references.is_empty()
        && references.iter().all(|reference| match reference {
            Json::Object(entries) => matches!(
                entries.get("require_approval"),
                Some(Json::Text(approval)) if approval == "never"
            ),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
        })
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn extract_tool_calls_from_response(response: &ResponsesApiResponse) -> Vec<McpToolCall> {
    response
        .output
        .iter()
        .filter(|item| is_function_call(*item))
        .map(|item| McpToolCall::Output(Box::new(item.clone())))
        .collect()
}

pub fn extract_tool_call_details(
    call: &McpToolCall,
) -> Result<McpToolCallDetails, ToolSelectionError> {
    match call {
        McpToolCall::Output(item) => {
            let call_id = function_call_id(item).map(str::to_owned);
            let (name, arguments) = output_details(item)?;
            Ok(McpToolCallDetails {
                name,
                arguments,
                call_id,
            })
        }
        McpToolCall::Fields(entries) => {
            let call_id = mapping_call_id(entries)?;
            let (name, arguments) = mapping_details(entries)?;
            Ok(McpToolCallDetails {
                name,
                arguments,
                call_id,
            })
        }
    }
}

fn is_function_call(item: &ResponseOutputItem) -> bool {
    match item {
        ResponseOutputItem::Sdk(output) => matches!(output.as_ref(), SdkOutput::FunctionCall(_)),
        ResponseOutputItem::FunctionCall(call) => call.r#type.as_deref() == Some("function_call"),
        ResponseOutputItem::Message(message) => message.r#type == "function_call",
        ResponseOutputItem::Unknown(entries) => {
            matches!(entries.get("type"), Some(Json::Text(kind)) if kind == "function_call")
        }
        ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => false,
    }
}

fn output_details(
    item: &ResponseOutputItem,
) -> Result<(Option<String>, Option<Json>), ToolSelectionError> {
    match item {
        ResponseOutputItem::Sdk(output) => {
            let SdkOutput::FunctionCall(call) = output.as_ref() else {
                return Ok((None, None));
            };
            if has_function_member(&call.extra) {
                return Ok((None, None));
            }
            Ok((
                Some(call.name.clone()),
                Some(Json::Text(call.arguments.clone())),
            ))
        }
        ResponseOutputItem::FunctionCall(call) => {
            if has_function_member(&call.extra) {
                return Ok((None, None));
            }
            let arguments = call
                .arguments
                .clone()
                .map(Json::Text)
                .or_else(|| value_before_null(call.extra.get("input")));
            Ok((call.name.clone(), arguments))
        }
        ResponseOutputItem::Message(message) => attribute_details(&message.extra),
        ResponseOutputItem::Unknown(entries) => mapping_details(entries),
        ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => Ok((None, None)),
    }
}

// a pydantic model answers getattr for an extra `function` key, so a non-null one hides the name and arguments
fn attribute_details(
    entries: &BTreeMap<String, Json>,
) -> Result<(Option<String>, Option<Json>), ToolSelectionError> {
    if has_function_member(entries) {
        return Ok((None, None));
    }
    let name = text_field(entries, "name")?;
    let arguments = value_before_null(entries.get("arguments"))
        .or_else(|| value_before_null(entries.get("input")));
    Ok((name, arguments))
}

fn mapping_details(
    entries: &BTreeMap<String, Json>,
) -> Result<(Option<String>, Option<Json>), ToolSelectionError> {
    if let Some(Json::Object(function)) = entries.get("function") {
        let name = text_field(function, "name")?;
        return Ok((name, value_before_null(function.get("arguments"))));
    }
    let name = text_field(entries, "name")?;
    let arguments = value_before_null(entries.get("arguments"))
        .or_else(|| value_before_null(entries.get("input")));
    Ok((name, arguments))
}

fn mapping_call_id(entries: &BTreeMap<String, Json>) -> Result<Option<String>, ToolSelectionError> {
    match text_field(entries, "call_id")? {
        Some(call_id) if !call_id.is_empty() => Ok(Some(call_id)),
        Some(_) | None => text_field(entries, "id"),
    }
}

fn text_field(
    entries: &BTreeMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, ToolSelectionError> {
    match entries.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ToolSelectionError::NotAString { field })
        }
    }
}

fn value_before_null(value: Option<&Json>) -> Option<Json> {
    match value {
        None | Some(Json::Null) => None,
        Some(found) => Some(found.clone()),
    }
}

fn has_function_member(entries: &BTreeMap<String, Json>) -> bool {
    match entries.get("function") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            true
        }
    }
}

mod tests;
