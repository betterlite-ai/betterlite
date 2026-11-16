use betterlite_context::agentic_loop_plan::FollowupMessage;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde_json::value::to_raw_value;

use super::error::RecoveryError;
use super::response::{ModelResponse, ResponseFormat};
use super::retrieval::Retrieved;
use super::tools::{CallKind, RetrieveCall};

// A second set of builders because headroom_retrieval's followup is private to that module; sharing it is out of this task's scope
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
}

#[derive(Serialize)]
struct AnthropicTurn {
    role: &'static str,
    content: Vec<AnthropicBlock>,
}

#[derive(Serialize)]
struct AnthropicResult {
    #[serde(rename = "type")]
    kind: &'static str,
    tool_use_id: Option<String>,
    content: String,
}

#[derive(Serialize)]
struct AnthropicAnswers {
    role: &'static str,
    content: Vec<AnthropicResult>,
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

// The original messages, then the assistant turn and one answer per retrieved call
pub(super) fn append(
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

fn followup<T>(value: &T) -> Result<FollowupMessage, RecoveryError>
where
    T: Serialize + ?Sized,
{
    match to_raw_value(value) {
        Ok(raw) => Ok(FollowupMessage(raw)),
        Err(source) => Err(RecoveryError::from(source)),
    }
}

fn arguments_json(call: &RetrieveCall) -> Result<String, RecoveryError> {
    match serde_json::to_string(&call.arguments) {
        Ok(encoded) => Ok(encoded),
        Err(source) => Err(RecoveryError::from(source)),
    }
}

// Other calls of the turn are left out since they have no answer, so the model plans them again
fn chat_messages(
    text: Option<String>,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RecoveryError> {
    let mut calls = Vec::with_capacity(retrieved.len());
    let mut results = Vec::with_capacity(retrieved.len());
    for entry in retrieved {
        calls.push(ChatCall {
            id: entry.call.id.clone(),
            kind: CallKind::Function,
            function: ChatFunction {
                name: entry.call.name.clone(),
                arguments: arguments_json(&entry.call)?,
            },
        });
        results.push(ChatResult {
            role: TOOL,
            tool_call_id: entry.call.id.clone(),
            content: entry.content.clone(),
        });
    }
    let assistant = ChatAssistant {
        role: ASSISTANT,
        content: text,
        tool_calls: calls,
    };
    let mut messages = Vec::with_capacity(results.len());
    messages.push(followup(&assistant)?);
    for result in &results {
        messages.push(followup(result)?);
    }
    Ok(messages)
}

fn anthropic_messages(
    text: Option<String>,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RecoveryError> {
    Ok(vec![
        anthropic_turn(text, retrieved)?,
        anthropic_answers(retrieved)?,
    ])
}

fn anthropic_turn(
    text: Option<String>,
    retrieved: &[Retrieved],
) -> Result<FollowupMessage, RecoveryError> {
    let mut blocks: Vec<AnthropicBlock> = Vec::with_capacity(retrieved.len());
    if let Some(said) = text {
        blocks.push(AnthropicBlock::Text { text: said });
    }
    for echo in retrieved {
        blocks.push(AnthropicBlock::ToolUse {
            id: echo.call.id.clone(),
            name: echo.call.name.clone(),
            input: echo.call.arguments.clone(),
        });
    }
    let turn = AnthropicTurn {
        role: ASSISTANT,
        content: blocks,
    };
    followup(&turn)
}

fn anthropic_answers(retrieved: &[Retrieved]) -> Result<FollowupMessage, RecoveryError> {
    let answers = AnthropicAnswers {
        role: USER,
        content: retrieved
            .iter()
            .map(|echo| AnthropicResult {
                kind: "tool_result",
                tool_use_id: echo.call.id.clone(),
                content: echo.content.clone(),
            })
            .collect(),
    };
    followup(&answers)
}

fn responses_items(
    text: Option<String>,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RecoveryError> {
    let mut items = Vec::new();
    if let Some(content) = text {
        items.push(followup(&ResponsesText {
            role: ASSISTANT,
            content,
        })?);
    }
    for record in retrieved {
        let call = ResponsesItem::FunctionCall {
            call_id: record.call.id.clone(),
            name: record.call.name.clone(),
            arguments: arguments_json(&record.call)?,
        };
        let output = ResponsesItem::FunctionCallOutput {
            call_id: record.call.id.clone(),
            output: record.content.clone(),
        };
        items.push(followup(&call)?);
        items.push(followup(&output)?);
    }
    Ok(items)
}
