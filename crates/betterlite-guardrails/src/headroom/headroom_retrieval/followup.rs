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

fn chat_tool_calls(retrieved: &[Retrieved]) -> Result<Vec<ChatToolCallOut>, RetrievalError> {
    retrieved
        .iter()
        .map(|entry| {
            Ok(ChatToolCallOut {
                id: entry.call.id.clone(),
                kind: FUNCTION_KIND,
                function: ChatFunctionOut {
                    name: entry.call.name.clone(),
                    arguments: arguments_json(&entry.call)?,
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
            if let Some(assistant) = responses_assistant(&response.view)? {
                follow_up.push(assistant);
            }
            for entry in retrieved {
                follow_up.push(responses_call(entry)?);
                follow_up.push(responses_output(entry)?);
            }
        }
        ResponseFormat::Anthropic => {
            follow_up.push(followup(&anthropic_assistant(&response.view, retrieved))?);
            follow_up.push(followup(&anthropic_user(retrieved))?);
        }
        ResponseFormat::Chat => {
            follow_up.push(followup(&chat_assistant(
                &response.view,
                chat_tool_calls(retrieved)?,
            ))?);
            for entry in retrieved {
                follow_up.push(chat_result(entry)?);
            }
        }
    }
    Ok(follow_up)
}

fn responses_assistant(view: &ResponseView) -> Result<Option<FollowupMessage>, RetrievalError> {
    assistant_text(view)
        .map(|content| {
            followup(&ResponsesAssistantOut {
                role: ASSISTANT_ROLE,
                content,
            })
        })
        .transpose()
}

fn responses_call(entry: &Retrieved) -> Result<FollowupMessage, RetrievalError> {
    followup(&ResponsesItem::FunctionCall {
        call_id: entry.call.id.clone(),
        name: entry.call.name.clone(),
        arguments: arguments_json(&entry.call)?,
    })
}

fn responses_output(entry: &Retrieved) -> Result<FollowupMessage, RetrievalError> {
    followup(&ResponsesItem::FunctionCallOutput {
        call_id: entry.call.id.clone(),
        output: entry.content.clone(),
    })
}

fn anthropic_assistant(view: &ResponseView, retrieved: &[Retrieved]) -> AnthropicAssistantOut {
    let mut content: Vec<AnthropicBlock> = retrieved.iter().map(anthropic_use).collect();
    if let Some(text) = assistant_text(view) {
        content.insert(0, AnthropicBlock::Text { text });
    }
    AnthropicAssistantOut {
        role: ASSISTANT_ROLE,
        content,
    }
}

fn anthropic_use(entry: &Retrieved) -> AnthropicBlock {
    AnthropicBlock::ToolUse {
        id: entry.call.id.clone(),
        name: entry.call.name.clone(),
        input: entry.call.arguments.clone(),
    }
}

fn anthropic_user(retrieved: &[Retrieved]) -> AnthropicUserOut {
    AnthropicUserOut {
        role: USER_ROLE,
        content: retrieved.iter().map(anthropic_result).collect(),
    }
}

fn anthropic_result(entry: &Retrieved) -> AnthropicToolResultOut {
    AnthropicToolResultOut {
        kind: "tool_result",
        tool_use_id: entry.call.id.clone(),
        content: entry.content.clone(),
    }
}

fn chat_assistant(view: &ResponseView, tool_calls: Vec<ChatToolCallOut>) -> ChatAssistantOut {
    ChatAssistantOut {
        role: ASSISTANT_ROLE,
        content: assistant_text(view),
        tool_calls,
    }
}

fn chat_result(entry: &Retrieved) -> Result<FollowupMessage, RetrievalError> {
    followup(&ChatToolResultOut {
        role: TOOL_ROLE,
        tool_call_id: entry.call.id.clone(),
        content: entry.content.clone(),
    })
}
