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

fn chat_tool_calls(retrieved: &[Retrieved]) -> Result<Vec<ChatToolCallOut>, RetrievalError> {
    retrieved
        .iter()
        .map(|retrieved| {
            Ok(ChatToolCallOut {
                id: retrieved.call.id.clone(),
                kind: FUNCTION_KIND,
                function: ChatFunctionOut {
                    name: retrieved.call.name.clone(),
                    arguments: arguments_json(&retrieved.call)?,
                },
            })
        })
        .collect()
}

pub(crate) fn append(
    response: &ModelResponse,
    messages: &[FollowupMessage],
    retrieved: &[Retrieved],
) -> Result<Vec<FollowupMessage>, RetrievalError> {
    let mut follow_up = messages.to_vec();
    match response.format() {
        ResponseFormat::Responses => {
            if let Some(content) = assistant_text(&response.view) {
                follow_up.push(followup(&ResponsesAssistantOut {
                    role: ASSISTANT_ROLE,
                    content,
                })?);
            }
            for retrieved in retrieved {
                follow_up.push(followup(&ResponsesItem::FunctionCall {
                    call_id: retrieved.call.id.clone(),
                    name: retrieved.call.name.clone(),
                    arguments: arguments_json(&retrieved.call)?,
                })?);
                follow_up.push(followup(&ResponsesItem::FunctionCallOutput {
                    call_id: retrieved.call.id.clone(),
                    output: retrieved.content.clone(),
                })?);
            }
        }
        ResponseFormat::Anthropic => {
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
            follow_up.push(followup(&AnthropicAssistantOut {
                role: ASSISTANT_ROLE,
                content: blocks,
            })?);
            let results = retrieved
                .iter()
                .map(|retrieved| AnthropicToolResultOut {
                    kind: "tool_result",
                    tool_use_id: retrieved.call.id.clone(),
                    content: retrieved.content.clone(),
                })
                .collect();
            follow_up.push(followup(&AnthropicUserOut {
                role: USER_ROLE,
                content: results,
            })?);
        }
        ResponseFormat::Chat => {
            follow_up.push(followup(&ChatAssistantOut {
                role: ASSISTANT_ROLE,
                content: assistant_text(&response.view),
                tool_calls: chat_tool_calls(retrieved)?,
            })?);
            for retrieved in retrieved {
                follow_up.push(followup(&ChatToolResultOut {
                    role: TOOL_ROLE,
                    tool_call_id: retrieved.call.id.clone(),
                    content: retrieved.content.clone(),
                })?);
            }
        }
    }
    Ok(follow_up)
}
