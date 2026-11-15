use betterlite_context::agentic_loop_plan::FollowupMessage;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde_json::value::to_raw_value;

use super::assistant_text::assistant_text;
use super::error::RetrievalError;
use super::response::{ModelResponse, ResponseFormat, ResponseView};
use super::tools::RetrieveCall;

const FUNCTION_KIND: &str = "function";
const ASSISTANT_ROLE: &str = "assistant";
const USER_ROLE: &str = "user";
const TOOL_ROLE: &str = "tool";

pub(crate) struct Retrieved {
    pub(crate) call: RetrieveCall,
    pub(crate) content: String,
}

#[derive(Serialize)]
struct ChatFunctionOut {
    name: Option<String>,
    arguments: String,
}

#[derive(Serialize)]
struct ChatToolCallOut {
    id: Option<String>,
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatFunctionOut,
}

#[derive(Serialize)]
struct ChatAssistantOut {
    role: &'static str,
    content: Option<String>,
    tool_calls: Vec<ChatToolCallOut>,
}

#[derive(Serialize)]
struct ChatToolResultOut {
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
struct AnthropicAssistantOut {
    role: &'static str,
    content: Vec<AnthropicBlock>,
}

#[derive(Serialize)]
struct AnthropicToolResultOut {
    #[serde(rename = "type")]
    kind: &'static str,
    tool_use_id: Option<String>,
    content: String,
}

#[derive(Serialize)]
struct AnthropicUserOut {
    role: &'static str,
    content: Vec<AnthropicToolResultOut>,
}

#[derive(Serialize)]
struct ResponsesAssistantOut {
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

fn followup<T>(value: &T) -> Result<FollowupMessage, RetrievalError>
where
    T: Serialize + ?Sized,
{
    match to_raw_value(value) {
        Ok(raw) => Ok(FollowupMessage(raw)),
        Err(source) => Err(RetrievalError::from(source)),
    }
}

fn arguments_json(call: &RetrieveCall) -> Result<String, RetrievalError> {
    match serde_json::to_string(&call.arguments) {
        Ok(encoded) => Ok(encoded),
        Err(source) => Err(RetrievalError::from(source)),
    }
}

pub(crate) fn append(
    response: &ModelResponse,
    messages: &[FollowupMessage],
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let mut follow_up = messages.to_vec();
    match response.format() {
        ResponseFormat::Responses => {
            follow_up.extend(responses_items(&response.view, retrieved)?);
        }
        ResponseFormat::Anthropic => {
            follow_up.extend(anthropic_messages(&response.view, retrieved)?);
        }
        ResponseFormat::Chat => {
            follow_up.extend(chat_messages(&response.view, retrieved)?);
        }
    }
    Ok(follow_up)
}

// LiteLLM emits the assistant text item first, then a call and an output per retrieve call
fn responses_items(
    view: &ResponseView,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let mut items = Vec::with_capacity(retrieved.len());
    if let Some(content) = assistant_text(view) {
        items.push(followup(&ResponsesAssistantOut {
            role: ASSISTANT_ROLE,
            content,
        })?);
    }
    for entry in retrieved {
        let call = ResponsesItem::FunctionCall {
            call_id: entry.call.id.clone(),
            name: entry.call.name.clone(),
            arguments: arguments_json(&entry.call)?,
        };
        let output = ResponsesItem::FunctionCallOutput {
            call_id: entry.call.id.clone(),
            output: entry.content.clone(),
        };
        items.push(followup(&call)?);
        items.push(followup(&output)?);
    }
    Ok(items)
}

// the assistant turn carries the text and every tool_use block; the user turn answers each call
fn anthropic_messages(
    view: &ResponseView,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let mut content: Vec<AnthropicBlock> = Vec::with_capacity(retrieved.len());
    if let Some(text) = assistant_text(view) {
        content.push(AnthropicBlock::Text { text });
    }
    for entry in retrieved {
        content.push(AnthropicBlock::ToolUse {
            id: entry.call.id.clone(),
            name: entry.call.name.clone(),
            input: entry.call.arguments.clone(),
        });
    }
    let user = AnthropicUserOut {
        role: USER_ROLE,
        content: retrieved
            .iter()
            .map(|entry| AnthropicToolResultOut {
                kind: "tool_result",
                tool_use_id: entry.call.id.clone(),
                content: entry.content.clone(),
            })
            .collect(),
    };
    Ok(vec![
        followup(&AnthropicAssistantOut {
            role: ASSISTANT_ROLE,
            content,
        })?,
        followup(&user)?,
    ])
}

// one assistant message holding every call, then one tool message per call id
fn chat_messages(
    view: &ResponseView,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let mut calls = Vec::with_capacity(retrieved.len());
    let mut results = Vec::with_capacity(retrieved.len());
    for entry in retrieved {
        let arguments = arguments_json(&entry.call)?;
        calls.push(ChatToolCallOut {
            id: entry.call.id.clone(),
            kind: FUNCTION_KIND,
            function: ChatFunctionOut {
                name: entry.call.name.clone(),
                arguments,
            },
        });
        results.push(ChatToolResultOut {
            role: TOOL_ROLE,
            tool_call_id: entry.call.id.clone(),
            content: entry.content.clone(),
        });
    }
    let assistant = ChatAssistantOut {
        role: ASSISTANT_ROLE,
        content: assistant_text(view),
        tool_calls: calls,
    };
    let mut messages = Vec::with_capacity(results.len());
    messages.push(followup(&assistant)?);
    for result in &results {
        messages.push(followup(result)?);
    }
    Ok(messages)
}
