use std::collections::BTreeSet;

use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::error::CustomToolsError;
use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};

const MAX_ARGUMENT_CHARS: usize = 1_000_000;

struct NameFields<'tool> {
    kind: &'tool str,
    name: &'tool str,
    tools: &'tool [Json],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionChunk,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_callers: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionChunk {
    pub name: String,
    pub description: String,
    pub parameters: ContentParameters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentParameters {
    #[serde(rename = "type")]
    kind: &'static str,
    properties: ContentProperties,
    required: [&'static str; 1],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ContentProperties {
    content: ContentProperty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ContentProperty {
    #[serde(rename = "type")]
    kind: &'static str,
    description: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolCall<'call> {
    pub call_id: &'call str,
    pub name: &'call str,
    pub arguments_or_input: &'call str,
    pub status: &'call str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolCallPayload {
    Custom { input: String },
    Function { arguments: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCallItem {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: String,
    pub call_id: String,
    pub name: String,
    pub status: String,
    #[serde(flatten)]
    pub payload: ToolCallPayload,
}

// pydantic reads an absent field as its default and rejects a present one of another type
fn text_or<'fields>(
    fields: &'fields IndexMap<String, Json>,
    key: &str,
    default: &'fields str,
) -> Option<&'fields str> {
    match fields.get(key) {
        None => Some(default),
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn name_fields(tool: &Json) -> Option<NameFields<'_>> {
    let Json::Object(fields) = tool else {
        return None;
    };
    let tools = match fields.get("tools") {
        None => &[],
        Some(Json::List(items)) => items.as_slice(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return None;
        }
    };
    Some(NameFields {
        kind: text_or(fields, "type", "")?,
        name: text_or(fields, "name", "")?,
        tools,
    })
}

pub fn extract_custom_tool_names(tools: &[Json]) -> BTreeSet<String> {
    let namespaces = tools
        .iter()
        .filter_map(name_fields)
        .filter(|fields| fields.kind == "namespace")
        .flat_map(|fields| fields.tools);
    tools
        .iter()
        .chain(namespaces)
        .filter_map(name_fields)
        .filter(|fields| fields.kind == "custom" && !fields.name.is_empty())
        .map(|fields| fields.name.to_owned())
        .collect()
}

#[must_use]
pub fn tool_call_item_id(item_type: &str, tool_id: &str) -> String {
    let prefix = match item_type {
        "function_call" => "fc",
        "custom_tool_call" => "ctc",
        _ => return tool_id.to_owned(),
    };
    if tool_id.is_empty() || tool_id.starts_with(prefix) {
        return tool_id.to_owned();
    }
    format!("{prefix}_{tool_id}")
}

#[must_use]
pub fn serialize_tool_call_arguments(raw_arguments: &Json, default: &str) -> String {
    match raw_arguments {
        Json::Text(text) if !text.is_empty() => text.clone(),
        Json::Null | Json::Text(_) => default.to_owned(),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => dumps(raw_arguments),
    }
}

// Python's str() of the decoded content; containers have no JSON-compatible str, so they keep JSON text
fn content_text(content: &Json) -> String {
    match content {
        Json::Text(text) => text.clone(),
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(_) | Json::List(_) | Json::Object(_) => dumps(content),
    }
}

pub fn unwrap_custom_arguments(
    arguments: &[u8],
    limits: ApiLimits,
) -> Result<String, CustomToolsError> {
    let boundary = ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: arguments.len(),
    });
    limits
        .check(boundary)
        .map_err(|source| CustomToolsError::ArgumentsTooLarge { source })?;
    let text = str::from_utf8(arguments)
        .map_err(|source| CustomToolsError::ArgumentsNotUtf8 { source })?;
    if text.chars().nth(MAX_ARGUMENT_CHARS).is_some() {
        return Ok(text.to_owned());
    }
    let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(text) else {
        return Ok(text.to_owned());
    };
    Ok(fields
        .get("content")
        .map_or_else(|| text.to_owned(), content_text))
}

pub fn tool_call_item(
    call: ToolCall<'_>,
    custom_names: &BTreeSet<String>,
    limits: ApiLimits,
) -> Result<ToolCallItem, CustomToolsError> {
    let custom = custom_names.contains(call.name);
    let (kind, payload) = if !custom {
        let arguments = call.arguments_or_input.to_owned();
        ("function_call", ToolCallPayload::Function { arguments })
    } else if call.status == "completed" {
        let input = unwrap_custom_arguments(call.arguments_or_input.as_bytes(), limits)?;
        ("custom_tool_call", ToolCallPayload::Custom { input })
    } else {
        (
            "custom_tool_call",
            ToolCallPayload::Custom {
                input: String::new(),
            },
        )
    };
    Ok(ToolCallItem {
        kind,
        id: tool_call_item_id(kind, call.call_id),
        call_id: call.call_id.to_owned(),
        name: call.name.to_owned(),
        status: call.status.to_owned(),
        payload,
    })
}

fn allowed_callers(value: Option<&Json>) -> Result<Option<Vec<String>>, CustomToolsError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => items
            .iter()
            .map(|item| match item {
                Json::Text(caller) => Ok(caller.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(CustomToolsError::AllowedCallers)
                }
            })
            .collect::<Result<_, _>>()
            .map(Some),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(CustomToolsError::AllowedCallers)
        }
    }
}

fn grammar_suffix(format: Option<&Json>) -> String {
    let Some(Json::Object(fields)) = format else {
        return String::new();
    };
    let (Some(syntax), Some(definition)) = (
        text_or(fields, "syntax", ""),
        text_or(fields, "definition", ""),
    ) else {
        return String::new();
    };
    if definition.is_empty() {
        return String::new();
    }
    format!("\n\nFormat:\n```{syntax}\n{definition}\n```")
}

fn text_of<'fields>(fields: &'fields IndexMap<String, Json>, key: &str) -> &'fields str {
    if let Some(Json::Text(text)) = fields.get(key) {
        text
    } else {
        ""
    }
}

pub fn custom_to_function_tool(
    tool: &IndexMap<String, Json>,
) -> Result<Option<FunctionTool>, CustomToolsError> {
    if text_of(tool, "type") != "custom" {
        return Ok(None);
    }
    let name = text_of(tool, "name");
    let description = format!(
        "{}{}",
        text_of(tool, "description"),
        grammar_suffix(tool.get("format"))
    );
    let content = ContentProperty {
        kind: "string",
        description: format!("The {name} content following the specified format"),
    };
    Ok(Some(FunctionTool {
        kind: "function",
        function: FunctionChunk {
            name: name.to_owned(),
            description,
            parameters: ContentParameters {
                kind: "object",
                properties: ContentProperties { content },
                required: ["content"],
            },
        },
        allowed_callers: allowed_callers(tool.get("allowed_callers"))?,
    }))
}
