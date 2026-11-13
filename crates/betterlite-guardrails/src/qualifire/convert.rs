use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::error::QualifireError;
use crate::generic_guardrail_contracts::GuardrailMessage;

pub type MessageRow = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ApiToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiToolCall {
    pub id: String,
    pub name: String,
    pub arguments: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiTool {
    pub name: String,
    pub description: String,
    pub parameters: BTreeMap<String, GuardrailValue>,
}

#[must_use]
pub fn message_rows(messages: &[GuardrailMessage]) -> Vec<MessageRow> {
    messages
        .iter()
        .map(|message| {
            let mut row = message.extra.clone();
            row.insert(
                "role".to_owned(),
                GuardrailValue::Text(message.role.clone()),
            );
            row
        })
        .collect()
}

pub(super) fn api_message(row: &MessageRow) -> Result<ApiMessage, QualifireError> {
    Ok(ApiMessage {
        role: value_or(row.get("role"), "role")?.unwrap_or_else(|| "user".to_owned()),
        content: content(row.get("content"))?,
        tool_call_id: value_or(row.get("tool_call_id"), "tool_call_id")?,
        tool_calls: tool_calls(row.get("tool_calls"))?,
    })
}

fn value_or<T>(
    value: Option<&GuardrailValue>,
    field: &'static str,
) -> Result<Option<T>, QualifireError>
where
    T: DeserializeOwned,
{
    let Some(present) = value else {
        return Ok(None);
    };
    if matches!(present, GuardrailValue::Null) {
        return Ok(None);
    }
    let encoded =
        serde_json::to_vec(present).map_err(|source| QualifireError::Encode { source })?;
    serde_json::from_slice(&encoded)
        .map(Some)
        .map_err(|_wrong_type| QualifireError::InvalidField { field })
}

fn content(value: Option<&GuardrailValue>) -> Result<String, QualifireError> {
    match value {
        None | Some(GuardrailValue::Null) => Ok(String::new()),
        Some(GuardrailValue::Text(text)) => Ok(text.clone()),
        Some(GuardrailValue::Array(items)) => text_parts(items),
        Some(other) => {
            serde_json::to_string(other).map_err(|source| QualifireError::Encode { source })
        }
    }
}

fn text_parts(items: &[GuardrailValue]) -> Result<String, QualifireError> {
    let mut parts: Vec<&str> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            GuardrailValue::Text(text) => parts.push(text),
            GuardrailValue::Object(fields) => {
                let is_text_part = matches!(
                    fields.get("type"),
                    Some(GuardrailValue::Text(kind)) if kind == "text"
                );
                if !is_text_part {
                    continue;
                }
                let text = match fields.get("text") {
                    None | Some(GuardrailValue::Null) => "",
                    Some(GuardrailValue::Text(text)) => text,
                    Some(_wrong) => return Err(QualifireError::InvalidField { field: "text" }),
                };
                parts.push(text);
            }
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_) => {}
        }
    }
    Ok(parts.join("\n"))
}

fn tool_calls(value: Option<&GuardrailValue>) -> Result<Option<Vec<ApiToolCall>>, QualifireError> {
    let Some(calls) = value_or::<Vec<GuardrailValue>>(value, "tool_calls")? else {
        return Ok(None);
    };
    let converted = calls
        .iter()
        .map(api_tool_call)
        .collect::<Result<Vec<ApiToolCall>, QualifireError>>()?;
    Ok((!converted.is_empty()).then_some(converted))
}

fn object<'value>(
    value: &'value GuardrailValue,
    field: &'static str,
) -> Result<&'value BTreeMap<String, GuardrailValue>, QualifireError> {
    let GuardrailValue::Object(fields) = value else {
        return Err(QualifireError::InvalidField { field });
    };
    Ok(fields)
}

fn api_tool_call(value: &GuardrailValue) -> Result<ApiToolCall, QualifireError> {
    let call = object(value, "tool_calls")?;
    let function = value_or::<BTreeMap<String, GuardrailValue>>(call.get("function"), "function")?;
    let definition = function.as_ref();
    let id = value_or(call.get("id"), "id")?.unwrap_or_default();
    let name =
        value_or(definition.and_then(|fields| fields.get("name")), "name")?.unwrap_or_default();
    let arguments = arguments(definition.and_then(|fields| fields.get("arguments")));
    Ok(ApiToolCall {
        id,
        name,
        arguments,
    })
}

// LiteLLM reads an absent or non-object arguments value as {}, and a string is parsed as JSON
fn arguments(value: Option<&GuardrailValue>) -> BTreeMap<String, GuardrailValue> {
    match value {
        Some(GuardrailValue::Text(text)) => serde_json::from_str(text).unwrap_or_default(),
        Some(GuardrailValue::Object(fields)) => fields.clone(),
        None | Some(GuardrailValue::Null) | Some(_wrong) => BTreeMap::new(),
    }
}

pub fn api_tools(value: Option<&GuardrailValue>) -> Result<Option<Vec<ApiTool>>, QualifireError> {
    let Some(present) = value else {
        return Ok(None);
    };
    let GuardrailValue::Array(tools) = present else {
        return Err(QualifireError::InvalidField { field: "tools" });
    };
    let mut converted = Vec::new();
    for tool in tools {
        if let Some(converted_tool) = api_tool(tool)? {
            converted.push(converted_tool);
        }
    }
    Ok((!converted.is_empty()).then_some(converted))
}

fn api_tool(value: &GuardrailValue) -> Result<Option<ApiTool>, QualifireError> {
    let fields = object(value, "tools")?;
    let Some(definition) = definition_fields(fields)? else {
        return Ok(None);
    };
    let encoded =
        serde_json::to_vec(definition).map_err(|source| QualifireError::Encode { source })?;
    let parsed: ToolDefinition = serde_json::from_slice(&encoded)
        .map_err(|_wrong_type| QualifireError::InvalidField { field: "tools" })?;
    Ok(Some(ApiTool {
        name: parsed.name.unwrap_or_default(),
        description: parsed.description.unwrap_or_default(),
        parameters: parsed.parameters.unwrap_or_default(),
    }))
}

#[derive(Debug, Deserialize)]
struct ToolDefinition {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Option<BTreeMap<String, GuardrailValue>>,
}

fn definition_fields(
    fields: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<&BTreeMap<String, GuardrailValue>>, QualifireError> {
    let is_function_tool = matches!(
        fields.get("type"),
        Some(GuardrailValue::Text(kind)) if kind == "function"
    );
    if is_function_tool {
        return match fields.get("function") {
            None | Some(GuardrailValue::Null) => Ok(None),
            Some(GuardrailValue::Object(function)) => Ok(Some(function)),
            Some(_wrong) => Err(QualifireError::InvalidField { field: "function" }),
        };
    }
    Ok(fields.contains_key("name").then_some(fields))
}
