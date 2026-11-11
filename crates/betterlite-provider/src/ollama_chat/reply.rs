use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use betterlite_streaming::stream_chunk_shaping::tokens::mapped_finish_reason;
use indexmap::IndexMap;

use self::decode::{DecodedReply, ReplyFunction, ReplyMessage};
use super::error::OllamaError;
use super::ids::IdSource;
use super::reasoning::parse_content_for_reasoning;
use super::usage::{TokenCounter, Usage};

pub mod decode;

const OLLAMA_CHAT_PREFIX: &str = "ollama_chat/";
const JSON_FORMAT: &str = "json";
const JSON_MODE_CALL_ID: &str = "call_";
const FUNCTION_TYPE: &str = "function";
const TOOL_CALLS_FINISH_REASON: &str = "tool_calls";
const DEFAULT_ROLE: &str = "assistant";
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
    let (message, finish_reason) = if json_mode(input) {
        json_mode_message(content.as_deref(), reasoning, input.litellm_params, ids)?
    } else {
        let mapped = mapped_finish_reason(done_reason(&reply));
        assistant_message(&reply.message, content, reasoning, mapped, ids)?
    };
    Ok(OllamaChatResponse {
        model: format!("{OLLAMA_CHAT_PREFIX}{}", input.model),
        created: input.created,
        choices: vec![ChatChoice {
            index: 0,
            message,
            finish_reason,
        }],
        usage,
    })
}

fn done_reason(reply: &DecodedReply) -> &str {
    match reply.done_reason.as_deref() {
        Some(text) if !text.is_empty() => text,
        Some(_) | None => DEFAULT_FINISH_REASON,
    }
}

// LiteLLM only parses the content for reasoning tags when the message carries no thinking text
fn message_reasoning(message: &ReplyMessage) -> (Option<String>, Option<String>) {
    message.thinking.as_ref().map_or_else(
        || {
            message.content.as_deref().map_or((None, None), |text| {
                let (reasoning, content) = parse_content_for_reasoning(text);
                (reasoning.map(str::to_owned), Some(content.to_owned()))
            })
        },
        |thinking| (Some(thinking.clone()), message.content.clone()),
    )
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
    message: &ReplyMessage,
    content: Option<String>,
    reasoning: Option<String>,
    mapped_finish: &str,
    ids: &dyn IdSource,
) -> Result<(AssistantMessage, String), OllamaError> {
    let tool_calls = reply_tool_calls(message, ids)?;
    let finish_reason = if tool_calls.is_some() {
        String::from(TOOL_CALLS_FINISH_REASON)
    } else {
        mapped_finish.to_owned()
    };
    Ok((
        AssistantMessage {
            role: message
                .role
                .clone()
                .unwrap_or_else(|| String::from(DEFAULT_ROLE)),
            content,
            reasoning_content: reasoning,
            tool_calls,
        },
        finish_reason,
    ))
}

fn reply_tool_calls(
    message: &ReplyMessage,
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
            kind: call
                .kind
                .clone()
                .unwrap_or_else(|| String::from(FUNCTION_TYPE)),
            function: call_function(&call.function)?,
        });
    }
    Ok(Some(converted))
}

fn call_function(function: &ReplyFunction) -> Result<Function, OllamaError> {
    let arguments = match &function.arguments {
        Some(value @ Json::Object(_)) => {
            serde_json::to_string(value).map_err(|source| OllamaError::Shape { source })?
        }
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) | None => parameters_arguments(function)?,
        Some(Json::List(_) | Json::Bool(_) | Json::Number(_)) => {
            return Err(OllamaError::ReplyField {
                field: "message.tool_calls.function.arguments",
            });
        }
    };
    Ok(Function {
        name: function.name.clone(),
        arguments,
    })
}

fn parameters_arguments(function: &ReplyFunction) -> Result<String, OllamaError> {
    match &function.parameters {
        Some(value @ Json::Object(_)) => {
            serde_json::to_string(value).map_err(|source| OllamaError::Shape { source })
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => Ok(String::new()),
    }
}

fn json_mode_message(
    content: Option<&str>,
    reasoning: Option<String>,
    litellm_params: &IndexMap<String, Json>,
    ids: &dyn IdSource,
) -> Result<(AssistantMessage, String), OllamaError> {
    let Some(text) = content else {
        return Err(OllamaError::ReplyField {
            field: "message.content",
        });
    };
    let (name, arguments) = json_mode_call(text, litellm_params)?;
    Ok((
        AssistantMessage {
            role: String::from(DEFAULT_ROLE),
            content: None,
            reasoning_content: reasoning,
            tool_calls: Some(vec![ToolCall {
                id: format!("{JSON_MODE_CALL_ID}{}", ids.next_id()),
                kind: String::from(FUNCTION_TYPE),
                function: Function { name, arguments },
            }]),
        },
        String::from(TOOL_CALLS_FINISH_REASON),
    ))
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
