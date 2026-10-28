use serde::{Deserialize, Serialize};

use crate::message_content::{ChatRole, content_text, role_for};

mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiField {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum UiContent {
    Messages { messages: Vec<UiMessage> },
    Fields { fields: Vec<UiField> },
    Text { text: String },
}

pub fn to_ui_content(raw: &str) -> UiContent {
    if raw.is_empty() {
        return UiContent::Text {
            text: String::new(),
        };
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return UiContent::Text {
            text: raw.to_owned(),
        };
    };
    if let Some(messages) = parse_messages(&value) {
        return UiContent::Messages { messages };
    }
    if let Some(object) = value.as_object() {
        let fields = object
            .iter()
            .map(|(key, value)| UiField {
                key: key.clone(),
                value: field_value(value),
            })
            .collect();
        return UiContent::Fields { fields };
    }
    UiContent::Text {
        text: value.as_str().unwrap_or(raw).to_owned(),
    }
}

fn parse_messages(value: &serde_json::Value) -> Option<Vec<UiMessage>> {
    let values = value
        .as_array()
        .cloned()
        .or_else(|| value.as_object().map(|_| vec![value.clone()]))?;
    if values.is_empty() {
        return None;
    }
    values.iter().map(parse_message).collect()
}

fn parse_message(value: &serde_json::Value) -> Option<UiMessage> {
    let object = value.as_object()?;
    let wrapped = object
        .get("kwargs")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(object);
    let role_name = wrapped
        .get("role")
        .or_else(|| wrapped.get("type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let calls = wrapped
        .get("tool_calls")
        .and_then(serde_json::Value::as_array)
        .map(|values| parse_calls(values))
        .unwrap_or_default();
    if !wrapped.contains_key("content") && calls.is_empty() {
        return None;
    }
    let role = role_for(role_name, !calls.is_empty());
    Some(UiMessage {
        role,
        content: content_text(wrapped.get("content").unwrap_or(&serde_json::Value::Null)),
        name: wrapped
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        tool_calls: (!calls.is_empty()).then_some(calls),
    })
}

fn parse_calls(values: &[serde_json::Value]) -> Vec<ToolCall> {
    let calls = values
        .iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let function = object
                .get("function")
                .and_then(serde_json::Value::as_object);
            let source = function.unwrap_or(object);
            let name = source
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            let arguments = source
                .get("arguments")
                .or_else(|| source.get("args"))
                .unwrap_or(&serde_json::Value::Null);
            Some(ToolCall {
                name,
                arguments: match arguments {
                    serde_json::Value::String(text) => text.clone(),
                    serde_json::Value::Null => "{}".to_owned(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                },
            })
        })
        .collect();
    calls
}

fn field_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}
