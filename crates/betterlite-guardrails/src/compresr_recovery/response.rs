use betterlite_prompts::response_tool_calls::projections::ToolCallResponse;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::error::RecoveryError;
use crate::primitives::json_value::JsonValue;

// The three surfaces LiteLLM probes, each kept whole so a list is told from a missing key
#[derive(Deserialize)]
pub(super) struct ResponseView {
    #[serde(default)]
    pub(super) choices: JsonValue,
    #[serde(default)]
    pub(super) content: JsonValue,
    #[serde(default)]
    pub(super) output: JsonValue,
}

// A second response view because headroom_retrieval's is private to that module; sharing it is out of this task's scope
pub(super) struct ModelResponse {
    pub(super) calls: ToolCallResponse,
    pub(super) view: ResponseView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResponseFormat {
    Responses,
    Anthropic,
    Chat,
}

impl ModelResponse {
    pub(super) fn parse(response: &RawValue) -> Result<Self, RecoveryError> {
        let encoded = response.get();
        let calls =
            serde_json::from_str(encoded).map_err(|source| RecoveryError::Response { source })?;
        let view =
            serde_json::from_str(encoded).map_err(|source| RecoveryError::Response { source })?;
        Ok(Self { calls, view })
    }

    // A list under `output` means Responses, a list under `content` means Anthropic, anything else is chat
    pub(super) const fn format(&self) -> ResponseFormat {
        if matches!(self.view.output, JsonValue::Array(_)) {
            ResponseFormat::Responses
        } else if matches!(self.view.content, JsonValue::Array(_)) {
            ResponseFormat::Anthropic
        } else {
            ResponseFormat::Chat
        }
    }

    // LiteLLM's assistant_text_from_response: chat first, then Anthropic blocks, then Responses message items
    pub(super) fn assistant_text(&self) -> Option<String> {
        chat_text(&self.view.choices)
            .or_else(|| text_blocks(&self.view.content, "text"))
            .or_else(|| responses_text(&self.view.output))
    }
}

fn items(value: &JsonValue) -> &[JsonValue] {
    match value {
        JsonValue::Array(found) => found,
        _ => &[],
    }
}

// The text of a block of the given type, empty strings included
fn typed_text<'block>(block: &'block JsonValue, kind: &str) -> Option<&'block str> {
    let JsonValue::Object(fields) = block else {
        return None;
    };
    if !matches!(fields.get("type"), Some(JsonValue::String(found)) if found == kind) {
        return None;
    }
    match fields.get("text") {
        Some(JsonValue::String(text)) => Some(text.as_str()),
        _ => None,
    }
}

// None when nothing joins, so an empty text never wins over the next response shape
fn text_blocks(value: &JsonValue, kind: &str) -> Option<String> {
    let kept: Vec<&str> = items(value)
        .iter()
        .filter_map(|block| typed_text(block, kind))
        .filter(|text| !text.is_empty())
        .collect();
    (!kept.is_empty()).then(|| kept.concat())
}

fn chat_text(choices: &JsonValue) -> Option<String> {
    let Some(JsonValue::Object(choice)) = items(choices).first() else {
        return None;
    };
    let Some(JsonValue::Object(message)) = choice.get("message") else {
        return None;
    };
    match message.get("content") {
        Some(JsonValue::String(whole)) => Some(whole.clone()).filter(|text| !text.is_empty()),
        Some(JsonValue::Array(parts)) => {
            let joined = parts
                .iter()
                .filter_map(|part| typed_text(part, "text"))
                .collect::<Vec<_>>()
                .join("\n\n");
            Some(joined).filter(|text| !text.is_empty())
        }
        _ => None,
    }
}

fn responses_text(output: &JsonValue) -> Option<String> {
    let mut text = String::new();
    for item in items(output) {
        let JsonValue::Object(fields) = item else {
            continue;
        };
        if !matches!(fields.get("type"), Some(JsonValue::String(found)) if found == "message") {
            continue;
        }
        let Some(content) = fields.get("content") else {
            continue;
        };
        for chunk in items(content) {
            if let Some(found) = typed_text(chunk, "output_text") {
                text.push_str(found);
            }
        }
    }
    (!text.is_empty()).then_some(text)
}
