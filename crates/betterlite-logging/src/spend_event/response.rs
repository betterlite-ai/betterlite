use serde::{Deserialize, Serialize};
use serde_with::{As, Same};

use crate::spend_event::input::ResponseObject;
use crate::spend_event::presence::Presence;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    ToolCalls,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Function,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolCall {
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: ToolKind,
    pub function: ToolCallFunction,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolCallMessage {
    pub role: MessageRole,
    pub content: (),
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolCallChoice {
    pub index: i64,
    pub finish_reason: FinishReason,
    pub message: ToolCallMessage,
}

// The spend pipeline reads a response's id, usage and called tools, never its content
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompactResponse {
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    pub id: Presence,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    pub model: Presence,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    pub usage: Presence,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    pub usage_info: Presence,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    pub status: Presence,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    pub background: Presence,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<ToolCallChoice>,
}

// Synthetic calls carry no id and empty arguments, in the order the extractor invoked them
fn tool_call_choice(names: &[String]) -> ToolCallChoice {
    let tool_calls = names
        .iter()
        .map(|name| ToolCall {
            id: None,
            kind: ToolKind::Function,
            function: ToolCallFunction {
                name: name.clone(),
                arguments: "{}".to_owned(),
            },
        })
        .collect();
    ToolCallChoice {
        index: 0,
        finish_reason: FinishReason::ToolCalls,
        message: ToolCallMessage {
            role: MessageRole::Assistant,
            content: (),
            tool_calls,
        },
    }
}

#[must_use]
pub fn compact_response(response: &ResponseObject) -> Option<CompactResponse> {
    let body = response.body()?;
    let pick = |key: &str| {
        body.fields
            .get(key)
            .cloned()
            .map_or(Presence::Missing, Presence::Present)
    };
    let choices = if body.tool_call_names.is_empty() {
        Vec::new()
    } else {
        vec![tool_call_choice(&body.tool_call_names)]
    };
    Some(CompactResponse {
        id: pick("id"),
        model: pick("model"),
        usage: pick("usage"),
        usage_info: pick("usage_info"),
        status: pick("status"),
        background: pick("background"),
        choices,
    })
}
