use betterlite_context::agentic_loop_plan::FollowupMessage;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde_json::value::to_raw_value;

use super::assistant_text::assistant_text;
use super::error::RetrievalError;
use super::response::{ModelResponse, ResponseFormat};
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

#[derive(Serialize)]
#[serde(untagged)]
enum ResponsesFollowupItem {
    Assistant(ResponsesAssistantOut),
    Item(ResponsesItem),
}

pub(crate) fn append(
    response: &ModelResponse,
    messages: &[FollowupMessage],
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let built = match response.format() {
        ResponseFormat::Responses => responses_messages(response, retrieved)?,
        ResponseFormat::Anthropic => anthropic_messages(response, retrieved)?,
        ResponseFormat::Chat => chat_messages(response, retrieved)?,
    };
    let mut follow_up = messages.to_vec();
    follow_up.extend(built);
    Ok(follow_up)
}

fn chat_messages(
    response: &ModelResponse,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let tool_calls = retrieved
        .iter()
        .map(|retrieved| {
            Ok(ChatToolCallOut {
                id: retrieved.call.id.clone(),
                kind: FUNCTION_KIND,
                function: ChatFunctionOut {
                    name: retrieved.call.name.clone(),
                    arguments: serde_json::to_string(&retrieved.call.arguments)?,
                },
            })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    let assistant = ChatAssistantOut {
        role: ASSISTANT_ROLE,
        content: assistant_text(&response.view),
        tool_calls,
    };
    let mut messages = vec![FollowupMessage(to_raw_value(&assistant)?)];
    for retrieved in retrieved {
        messages.push(FollowupMessage(to_raw_value(&ChatToolResultOut {
            role: TOOL_ROLE,
            tool_call_id: retrieved.call.id.clone(),
            content: retrieved.content.clone(),
        })?));
    }
    Ok(messages)
}

fn anthropic_messages(
    response: &ModelResponse,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let mut blocks = Vec::new();
    if let Some(text) = assistant_text(&response.view) {
        blocks.push(AnthropicBlock::Text { text });
    }
    for retrieved in retrieved {
        blocks.push(AnthropicBlock::ToolUse {
            id: retrieved.call.id.clone(),
            name: retrieved.call.name.clone(),
            input: retrieved.call.arguments.clone(),
        });
    }
    let assistant = AnthropicAssistantOut {
        role: ASSISTANT_ROLE,
        content: blocks,
    };
    let results = retrieved
        .iter()
        .map(|retrieved| AnthropicToolResultOut {
            kind: "tool_result",
            tool_use_id: retrieved.call.id.clone(),
            content: retrieved.content.clone(),
        })
        .collect();
    let user = AnthropicUserOut {
        role: USER_ROLE,
        content: results,
    };
    Ok(vec![
        FollowupMessage(to_raw_value(&assistant)?),
        FollowupMessage(to_raw_value(&user)?),
    ])
}

fn responses_messages(
    response: &ModelResponse,
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let mut items = Vec::new();
    if let Some(text) = assistant_text(&response.view) {
        items.push(ResponsesFollowupItem::Assistant(ResponsesAssistantOut {
            role: ASSISTANT_ROLE,
            content: text,
        }));
    }
    for retrieved in retrieved {
        items.push(ResponsesFollowupItem::Item(ResponsesItem::FunctionCall {
            call_id: retrieved.call.id.clone(),
            name: retrieved.call.name.clone(),
            arguments: serde_json::to_string(&retrieved.call.arguments)?,
        }));
        items.push(ResponsesFollowupItem::Item(
            ResponsesItem::FunctionCallOutput {
                call_id: retrieved.call.id.clone(),
                output: retrieved.content.clone(),
            },
        ));
    }
    items
        .iter()
        .map(|item| Ok(FollowupMessage(to_raw_value(item)?)))
        .collect()
}
