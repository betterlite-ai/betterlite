use betterlite_context::agentic_loop_plan::FollowupMessage;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde_json::value::to_raw_value;

use super::error::RecoveryError;
use super::response::{ModelResponse, ResponseFormat};
use super::retrieval::Retrieved;
use super::tools::CallKind;

const ASSISTANT: &str = "assistant";
const USER: &str = "user";
const TOOL: &str = "tool";

#[derive(Serialize)]
struct ChatFunction {
    name: Option<String>,
    arguments: String,
}

#[derive(Serialize)]
struct ChatCall {
    id: Option<String>,
    #[serde(rename = "type")]
    kind: CallKind,
    function: ChatFunction,
}

#[derive(Serialize)]
struct ChatAssistant {
    role: &'static str,
    content: Option<String>,
    tool_calls: Vec<ChatCall>,
}

#[derive(Serialize)]
struct ChatResult {
    role: &'static str,
    tool_call_id: Option<String>,
    content: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: Option<String>,
        name: Option<String>,
        input: Object,
    },
    ToolResult {
        tool_use_id: Option<String>,
        content: String,
    },
}

#[derive(Serialize)]
struct AnthropicTurn {
    role: &'static str,
    content: Vec<AnthropicBlock>,
}

#[derive(Serialize)]
struct ResponsesText {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponsesItem {
    FunctionCall {
        call_id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: Option<String>,
        output: String,
    },
}

// The original messages, then the retrieve calls the model made with their recovered content
pub fn append(
    response: &ModelResponse,
    messages: &[FollowupMessage],
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RecoveryError> {
    let text = response.assistant_text();
    let additions = match response.format() {
        ResponseFormat::Responses => responses_items(text, retrieved)?,
        ResponseFormat::Anthropic => anthropic_messages(text, retrieved)?,
        ResponseFormat::Chat => chat_messages(text, retrieved)?,
    };
    let mut follow_up = messages.to_vec();
    follow_up.extend(additions);
    Ok(follow_up)
}

// Other calls of the turn are left out since they have no result, so the model plans them again
fn chat_messages(
    text: Option<String>,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RecoveryError> {
    let mut tool_calls = Vec::with_capacity(retrieved.len());
    let mut results = Vec::with_capacity(retrieved.len());
    for entry in retrieved {
        tool_calls.push(ChatCall {
            id: entry.call.id.clone(),
            kind: CallKind::Function,
            function: ChatFunction {
                name: entry.call.name.clone(),
                arguments: serde_json::to_string(&entry.call.arguments)?,
            },
        });
        let result = ChatResult {
            role: TOOL,
            tool_call_id: entry.call.id.clone(),
            content: entry.content.clone(),
        };
        results.push(FollowupMessage(to_raw_value(&result)?));
    }
    let assistant = ChatAssistant {
        role: ASSISTANT,
        content: text,
        tool_calls,
    };
    let mut messages = vec![FollowupMessage(to_raw_value(&assistant)?)];
    messages.extend(results);
    Ok(messages)
}

fn anthropic_messages(
    text: Option<String>,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RecoveryError> {
    let mut calls: Vec<AnthropicBlock> = text
        .into_iter()
        .map(|content| AnthropicBlock::Text { text: content })
        .collect();
    let mut results = Vec::with_capacity(retrieved.len());
    for entry in retrieved {
        calls.push(AnthropicBlock::ToolUse {
            id: entry.call.id.clone(),
            name: entry.call.name.clone(),
            input: entry.call.arguments.clone(),
        });
        results.push(AnthropicBlock::ToolResult {
            tool_use_id: entry.call.id.clone(),
            content: entry.content.clone(),
        });
    }
    let assistant = AnthropicTurn {
        role: ASSISTANT,
        content: calls,
    };
    let user = AnthropicTurn {
        role: USER,
        content: results,
    };
    Ok(vec![
        FollowupMessage(to_raw_value(&assistant)?),
        FollowupMessage(to_raw_value(&user)?),
    ])
}

fn responses_items(
    text: Option<String>,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RecoveryError> {
    let mut items = Vec::new();
    if let Some(content) = text {
        let assistant = ResponsesText {
            role: ASSISTANT,
            content,
        };
        items.push(FollowupMessage(to_raw_value(&assistant)?));
    }
    for entry in retrieved {
        let call = ResponsesItem::FunctionCall {
            call_id: entry.call.id.clone(),
            name: entry.call.name.clone(),
            arguments: serde_json::to_string(&entry.call.arguments)?,
        };
        let output = ResponsesItem::FunctionCallOutput {
            call_id: entry.call.id.clone(),
            output: entry.content.clone(),
        };
        items.push(FollowupMessage(to_raw_value(&call)?));
        items.push(FollowupMessage(to_raw_value(&output)?));
    }
    Ok(items)
}
