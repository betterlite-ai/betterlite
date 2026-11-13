use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

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

pub fn api_messages(rows: &[MessageRow]) -> Result<Vec<ApiMessage>, QualifireError> {
    rows.iter().map(api_message).collect()
}

fn api_message(row: &MessageRow) -> Result<ApiMessage, QualifireError> {
    Ok(ApiMessage {
        role: text_field(row, "role")?.unwrap_or_else(|| "user".to_owned()),
        content: content(row.get("content"))?,
        tool_call_id: text_field(row, "tool_call_id")?,
        tool_calls: tool_calls(row.get("tool_calls"))?,
    })
}

fn text_field(row: &MessageRow, field: &'static str) -> Result<Option<String>, QualifireError> {
    match row.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_wrong) => Err(QualifireError::InvalidField { field }),
    }
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
    match value {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Array(calls)) => {
            let converted = calls
                .iter()
                .map(api_tool_call)
                .collect::<Result<Vec<ApiToolCall>, QualifireError>>()?;
            Ok((!converted.is_empty()).then_some(converted))
        }
        Some(_wrong) => Err(QualifireError::InvalidField {
            field: "tool_calls",
        }),
    }
}

fn api_tool_call(value: &GuardrailValue) -> Result<ApiToolCall, QualifireError> {
    let GuardrailValue::Object(call) = value else {
        return Err(QualifireError::InvalidField {
            field: "tool_calls",
        });
    };
    let function = match call.get("function") {
        None | Some(GuardrailValue::Null) => None,
        Some(GuardrailValue::Object(fields)) => Some(fields),
        Some(_wrong) => return Err(QualifireError::InvalidField { field: "function" }),
    };
    let arguments = match function.and_then(|fields| fields.get("arguments")) {
        None | Some(GuardrailValue::Null) => BTreeMap::new(),
        Some(GuardrailValue::Text(text)) => serde_json::from_str(text).unwrap_or_default(),
        Some(GuardrailValue::Object(fields)) => fields.clone(),
        Some(_wrong) => BTreeMap::new(),
    };
    Ok(ApiToolCall {
        id: plain_field(call.get("id"), "id")?,
        name: plain_field(function.and_then(|fields| fields.get("name")), "name")?,
        arguments,
    })
}

fn plain_field(
    value: Option<&GuardrailValue>,
    field: &'static str,
) -> Result<String, QualifireError> {
    match value {
        None | Some(GuardrailValue::Null) => Ok(String::new()),
        Some(GuardrailValue::Text(text)) => Ok(text.clone()),
        Some(_wrong) => Err(QualifireError::InvalidField { field }),
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
    let GuardrailValue::Object(fields) = value else {
        return Err(QualifireError::InvalidField { field: "tools" });
    };
    let is_function_tool = matches!(
        fields.get("type"),
        Some(GuardrailValue::Text(kind)) if kind == "function"
    );
    let resolved = if is_function_tool {
        match fields.get("function") {
            None | Some(GuardrailValue::Null) => None,
            Some(GuardrailValue::Object(function)) => Some(function),
            Some(_wrong) => return Err(QualifireError::InvalidField { field: "function" }),
        }
    } else if fields.contains_key("name") {
        Some(fields)
    } else {
        None
    };
    let Some(definition) = resolved else {
        return Ok(None);
    };
    Ok(Some(ApiTool {
        name: plain_field(definition.get("name"), "name")?,
        description: plain_field(definition.get("description"), "description")?,
        parameters: object_field(definition.get("parameters"), "parameters")?,
    }))
}

fn object_field(
    value: Option<&GuardrailValue>,
    field: &'static str,
) -> Result<BTreeMap<String, GuardrailValue>, QualifireError> {
    match value {
        None | Some(GuardrailValue::Null) => Ok(BTreeMap::new()),
        Some(GuardrailValue::Object(fields)) => Ok(fields.clone()),
        Some(_wrong) => Err(QualifireError::InvalidField { field }),
    }
}
