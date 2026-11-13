use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::reasoning_fields::content_split::parse_content_for_reasoning;
use betterlite_streaming::stream_chunk_shaping::tokens::mapped_finish_reason;
use indexmap::IndexMap;

use self::decode::DecodedReply;
use super::error::OllamaError;
use super::function::{FUNCTION_TYPE, Thinking, WireMessage};
use super::ids::IdSource;
use super::usage::{TokenCounter, Usage};

pub mod decode;

const OLLAMA_CHAT_PREFIX: &str = "ollama_chat/";
const JSON_FORMAT: &str = "json";
const JSON_MODE_CALL_ID: &str = "call_";
const TOOL_CALLS_FINISH_REASON: &str = "tool_calls";
const DEFAULT_FINISH_REASON: &str = "stop";

pub struct ReplyInput<'request> {
    pub model: &'request str,
    pub body: &'request [u8],
    pub request_data: &'request IndexMap<String, Json>,
    pub litellm_params: &'request IndexMap<String, Json>,
    pub messages: &'request [Message],
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaChatResponse {
    pub model: String,
    pub created: i64,
    pub choices: Vec<ChatChoice>,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatChoice {
    pub index: u32,
    pub message: AssistantMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantMessage {
    pub role: String,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub kind: String,
    pub function: Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: Option<String>,
    pub arguments: String,
}

pub async fn transform_response(
    input: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
    ids: &dyn IdSource,
) -> Result<OllamaChatResponse, OllamaError> {
    let reply = DecodedReply::try_from(input.body)?;
    let (reasoning, content) = message_reasoning(&reply.message);
    let usage = usage_for(&reply, content.as_deref(), input.messages, counter).await;
    let mapped = mapped_finish_reason(
        reply
            .done_reason
            .as_deref()
            .unwrap_or(DEFAULT_FINISH_REASON),
    );
    let json = json_mode(input).then_some(input.litellm_params);
    let (message, override_finish) =
        assistant_message(&reply.message, content, reasoning, json, ids)?;
    Ok(OllamaChatResponse {
        model: format!("{OLLAMA_CHAT_PREFIX}{}", input.model),
        created: input.created,
        choices: vec![ChatChoice {
            index: 0,
            message,
            finish_reason: override_finish.map_or_else(|| mapped.to_owned(), str::to_owned),
        }],
        usage,
    })
}

// LiteLLM parses the content for reasoning tags only when the message has no thinking key at all
fn message_reasoning(message: &WireMessage) -> (Option<String>, Option<String>) {
    match &message.thinking {
        Thinking::Absent => parse_content_for_reasoning(message.content.as_deref()),
        Thinking::Null => (None, message.content.clone()),
        Thinking::Text(thinking) => (Some(thinking.clone()), message.content.clone()),
    }
}

fn json_mode(input: &ReplyInput<'_>) -> bool {
    let format_is_json = matches!(
        input.request_data.get("format"),
        Some(Json::Text(format)) if format == JSON_FORMAT
    );
    let has_function_name = input
        .litellm_params
        .get("function_name")
        .is_some_and(|value| !matches!(value, Json::Null));
    format_is_json && has_function_name
}

fn assistant_message(
    message: &WireMessage,
    content: Option<String>,
    reasoning: Option<String>,
    json: Option<&IndexMap<String, Json>>,
    ids: &dyn IdSource,
) -> Result<(AssistantMessage, Option<&'static str>), OllamaError> {
    let role = message.role.clone();
    if let Some(litellm_params) = json {
        let built = json_mode_message(role, content.as_deref(), reasoning, litellm_params, ids)?;
        return Ok((built, Some(TOOL_CALLS_FINISH_REASON)));
    }
    let tool_calls = reply_tool_calls(message, ids)?;
    let override_finish = tool_calls.as_ref().map(|_calls| TOOL_CALLS_FINISH_REASON);
    Ok((
        AssistantMessage {
            role,
            content,
            reasoning_content: reasoning,
            tool_calls,
        },
        override_finish,
    ))
}

fn reply_tool_calls(
    message: &WireMessage,
    ids: &dyn IdSource,
) -> Result<Option<Vec<ToolCall>>, OllamaError> {
    let Some(calls) = message
        .tool_calls
        .as_ref()
        .filter(|calls| !calls.is_empty())
    else {
        return Ok(None);
    };
    let mut converted = Vec::with_capacity(calls.len());
    for call in calls {
        converted.push(ToolCall {
            id: call.id.clone().unwrap_or_else(|| ids.next_id()),
            kind: String::from(call.kind.as_deref().unwrap_or(FUNCTION_TYPE)),
            function: Function {
                name: call.function.name.clone(),
                arguments: call.function.call_arguments()?.arguments,
            },
        });
    }
    Ok(Some(converted))
}

fn json_mode_message(
    role: String,
    content: Option<&str>,
    reasoning: Option<String>,
    litellm_params: &IndexMap<String, Json>,
    ids: &dyn IdSource,
) -> Result<AssistantMessage, OllamaError> {
    let Some(text) = content else {
        return Err(OllamaError::ReplyField {
            field: "message.content",
        });
    };
    let (name, arguments) = json_mode_call(text, litellm_params)?;
    Ok(AssistantMessage {
        role,
        content: None,
        reasoning_content: reasoning,
        tool_calls: Some(vec![ToolCall {
            id: format!("{JSON_MODE_CALL_ID}{}", ids.next_id()),
            kind: String::from(FUNCTION_TYPE),
            function: Function { name, arguments },
        }]),
    })
}

fn json_mode_call(
    text: &str,
    litellm_params: &IndexMap<String, Json>,
) -> Result<(Option<String>, String), OllamaError> {
    let parsed =
        serde_json::from_str::<Json>(text).map_err(|source| OllamaError::Shape { source })?;
    let Json::Object(fields) = parsed else {
        return Err(OllamaError::ReplyField {
            field: "message.content",
        });
    };
    let name = match fields.get("name") {
        Some(Json::Text(name)) => Some(name.clone()),
        Some(Json::Null) => None,
        None => default_function_name(litellm_params)?,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(OllamaError::ReplyField {
                field: "message.content.name",
            });
        }
    };
    let argument_value = fields
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Json::Object(fields));
    let arguments =
        serde_json::to_string(&argument_value).map_err(|source| OllamaError::Shape { source })?;
    Ok((name, arguments))
}

fn default_function_name(
    litellm_params: &IndexMap<String, Json>,
) -> Result<Option<String>, OllamaError> {
    match litellm_params.get("function_name") {
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(Json::Null) | None => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(OllamaError::ReplyField {
                field: "litellm_params.function_name",
            })
        }
    }
}

async fn usage_for(
    reply: &DecodedReply,
    content: Option<&str>,
    messages: &[Message],
    counter: &dyn TokenCounter,
) -> Usage {
    let prompt_tokens = match reply.prompt_eval_count {
        Some(count) => count,
        None => counter.count_messages(messages).await.unwrap_or(0),
    };
    let completion_tokens = match reply.eval_count {
        Some(count) => count,
        None => match content {
            Some(text) => counter.count_text(text).await.unwrap_or(0),
            None => 0,
        },
    };
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
    }
}
