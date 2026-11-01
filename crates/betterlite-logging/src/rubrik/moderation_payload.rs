use serde::Serialize;
use uuid::Uuid;

use crate::rubrik::tool_calls::ChatCompletionMessageToolCall;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResponseModerationPayload {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ResponseModerationChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResponseModerationChoice {
    pub index: u32,
    pub message: ResponseModerationMessage,
    pub finish_reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResponseModerationMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ChatCompletionMessageToolCall>,
}

// Collapses the response's text segments into one string for the after_completion webhook to moderate; empty when the assistant said nothing.
#[must_use]
pub fn join_texts(texts: &[String]) -> String {
    let mut non_empty_texts = Vec::with_capacity(texts.len());
    for text in texts {
        if !text.is_empty() {
            non_empty_texts.push(text.as_str());
        }
    }
    non_empty_texts.join("\n")
}

#[must_use]
pub fn build_response_moderation(
    tool_calls: &[ChatCompletionMessageToolCall],
    content: &str,
    request_id: Option<&str>,
    created_unix_timestamp: i64,
) -> ResponseModerationPayload {
    let message = ResponseModerationMessage {
        role: "assistant".to_owned(),
        content: non_empty(content).map(str::to_owned),
        tool_calls: tool_calls.to_vec(),
    };
    let finish_reason = if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    ResponseModerationPayload {
        id: request_id
            .and_then(non_empty)
            .map_or_else(|| format!("chatcmpl-{}", Uuid::new_v4()), str::to_owned),
        object: "chat.completion".to_owned(),
        created: created_unix_timestamp,
        model: String::new(),
        choices: vec![ResponseModerationChoice {
            index: 0,
            message,
            finish_reason: finish_reason.to_owned(),
        }],
    }
}

const fn non_empty(text: &str) -> Option<&str> {
    if text.is_empty() { None } else { Some(text) }
}
