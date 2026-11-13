use betterlite_models::decisions::{Json, TokenCount};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClaudeMappingError {
    #[error("the response is not a chat completion")]
    Response(#[source] serde_json::Error),
    #[error("the response has no choices")]
    NoChoices,
}

#[derive(Debug, Deserialize)]
struct Completion {
    id: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ToolCall {
    id: String,
    #[serde(default)]
    function: Option<Function>,
    #[serde(default)]
    custom: Option<Custom>,
}

#[derive(Debug, Deserialize)]
struct Function {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct Custom {
    name: String,
    input: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: TokenCount,
    completion_tokens: TokenCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub role: &'static str,
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: ClaudeUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeUsage {
    pub input_tokens: TokenCount,
    pub output_tokens: TokenCount,
}

// Tool calls win over text, and a tool call with neither function nor custom is skipped.
fn content_blocks(message: Message) -> Vec<ContentBlock> {
    match message.tool_calls.filter(|calls| !calls.is_empty()) {
        Some(calls) => calls
            .into_iter()
            .filter_map(|call| match (call.function, call.custom) {
                (Some(function), _) => Some(ContentBlock::ToolUse {
                    id: call.id,
                    name: function.name,
                    input: function.arguments,
                }),
                (None, Some(custom)) => Some(ContentBlock::ToolUse {
                    id: call.id,
                    name: custom.name,
                    input: custom.input,
                }),
                (None, None) => None,
            })
            .collect(),
        None => message
            .content
            .filter(|text| !text.is_empty())
            .map(|text| vec![ContentBlock::Text { text }])
            .unwrap_or_default(),
    }
}

// LiteLLM also builds a Human/Assistant prompt string here and never sends it, so it is not built.
pub fn map(model: &str, response: &Json) -> Result<ClaudeMessage, ClaudeMappingError> {
    let value = serde_json::to_value(response).map_err(ClaudeMappingError::Response)?;
    let completion: Completion =
        serde_json::from_value(value).map_err(ClaudeMappingError::Response)?;
    let choice = completion
        .choices
        .into_iter()
        .next()
        .ok_or(ClaudeMappingError::NoChoices)?;
    Ok(ClaudeMessage {
        id: completion.id,
        kind: "message",
        role: "assistant",
        model: model.to_owned(),
        content: content_blocks(choice.message),
        stop_reason: choice.finish_reason,
        stop_sequence: None,
        usage: ClaudeUsage {
            input_tokens: completion.usage.prompt_tokens,
            output_tokens: completion.usage.completion_tokens,
        },
    })
}

mod tests;
