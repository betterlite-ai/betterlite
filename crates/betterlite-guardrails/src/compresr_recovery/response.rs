use betterlite_prompts::response_tool_calls::projections::ToolCallResponse;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::error::RecoveryError;
use crate::primitives::json_value::JsonValue;

// The three surfaces LiteLLM probes, each kept whole so a list is told from a missing key
#[derive(Deserialize)]
struct ResponseView {
    #[serde(default)]
    choices: JsonValue,
    #[serde(default)]
    content: JsonValue,
    #[serde(default)]
    output: JsonValue,
}

pub struct ModelResponse {
    pub calls: ToolCallResponse,
    view: ResponseView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    Responses,
    Anthropic,
    Chat,
}

impl ModelResponse {
    pub fn parse(response: &RawValue) -> Result<Self, RecoveryError> {
        let encoded = response.get();
        let calls =
            serde_json::from_str(encoded).map_err(|source| RecoveryError::Response { source })?;
        let view =
            serde_json::from_str(encoded).map_err(|source| RecoveryError::Response { source })?;
        Ok(Self { calls, view })
    }

    // A list under `output` means Responses, a list under `content` means Anthropic, anything else is chat
    #[must_use]
    pub const fn format(&self) -> ResponseFormat {
        if matches!(self.view.output, JsonValue::Array(_)) {
            ResponseFormat::Responses
        } else if matches!(self.view.content, JsonValue::Array(_)) {
            ResponseFormat::Anthropic
        } else {
            ResponseFormat::Chat
        }
    }

    // LiteLLM's assistant_text_from_response: chat first, then Anthropic blocks, then Responses message items
    #[must_use]
    pub fn assistant_text(&self) -> Option<String> {
        chat_text(&self.view.choices)
            .or_else(|| joined_blocks(&self.view.content, "text"))
            .or_else(|| responses_text(&self.view.output))
    }
}

fn field<'value>(value: &'value JsonValue, key: &str) -> Option<&'value JsonValue> {
    let JsonValue::Object(fields) = value else {
        return None;
    };
    fields.get(key)
}

fn items(value: &JsonValue) -> &[JsonValue] {
    if let JsonValue::Array(found) = value {
        found
    } else {
        &[]
    }
}

// The text of a block of the given type, empty strings included
fn typed_text<'block>(block: &'block JsonValue, kind: &str) -> Option<&'block str> {
    let JsonValue::String(found) = field(block, "type")? else {
        return None;
    };
    if found != kind {
        return None;
    }
    let JsonValue::String(text) = field(block, "text")? else {
        return None;
    };
    Some(text)
}

fn joined_blocks(blocks: &JsonValue, kind: &str) -> Option<String> {
    let texts: Vec<&str> = items(blocks)
        .iter()
        .filter_map(|block| typed_text(block, kind))
        .filter(|text| !text.is_empty())
        .collect();
    (!texts.is_empty()).then(|| texts.concat())
}

fn chat_text(choices: &JsonValue) -> Option<String> {
    let message = field(items(choices).first()?, "message")?;
    let text = match field(message, "content") {
        Some(JsonValue::String(whole)) => whole.clone(),
        Some(JsonValue::Array(parts)) => parts
            .iter()
            .filter_map(|part| typed_text(part, "text"))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Some(
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_),
        )
        | None => String::new(),
    };
    (!text.is_empty()).then_some(text)
}

fn responses_text(output: &JsonValue) -> Option<String> {
    let texts: Vec<&str> = items(output)
        .iter()
        .filter(|item| matches!(field(item, "type"), Some(JsonValue::String(kind)) if kind == "message"))
        .flat_map(|item| field(item, "content").map_or(&[][..], items))
        .filter_map(|chunk| typed_text(chunk, "output_text"))
        .filter(|text| !text.is_empty())
        .collect();
    (!texts.is_empty()).then(|| texts.concat())
}
