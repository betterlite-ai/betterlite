use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::reasoning_fields::content_split::parse_content_for_reasoning;
use indexmap::IndexMap;
use serde::Deserialize;

use super::error::OllamaGenerateError;
use crate::ollama_chat::error::OllamaError;
use crate::ollama_chat::function::FUNCTION_TYPE;
use crate::ollama_chat::ids::IdSource;
use crate::ollama_chat::reply::{
    AssistantMessage, ChatChoice, Function, OllamaChatResponse, ToolCall,
};
use crate::ollama_chat::usage::{TokenCounter, Usage};

const ARGUMENTS_FIELD: &str = "arguments";
const ASSISTANT_ROLE: &str = "assistant";
const JSON_FORMAT: &str = "json";
const NAME_FIELD: &str = "name";
const OLLAMA_PREFIX: &str = "ollama/";
const STOP_FINISH_REASON: &str = "stop";
const TOOL_CALLS_FINISH_REASON: &str = "tool_calls";

pub struct ReplyInput<'request> {
    pub model: &'request str,
    pub body: &'request [u8],
    pub request_data: &'request IndexMap<String, Json>,
    pub created: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GenerateReply {
    #[serde(default)]
    pub response: ResponseText,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub prompt_eval_count: OptionalCount,
    #[serde(default)]
    pub eval_count: OptionalCount,
    #[serde(default)]
    pub message: Option<ReplyMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyMessage {
    #[serde(default)]
    pub content: ResponseText,
}

// an absent count falls back to the counter, while an explicit null is a reply-field error
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(untagged)]
pub enum OptionalCount {
    #[default]
    #[serde(skip)]
    Absent,
    Null,
    Count(u64),
}

// an absent response stays "" while an explicit null stays None, so the two keep distinct variants
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ResponseText {
    #[default]
    #[serde(skip)]
    Absent,
    Null,
    Text(String),
}

impl ResponseText {
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Absent | Self::Null => "",
        }
    }
}

// the top-level thinking field wins over inline tags; a null response carries no text at all
#[must_use]
pub fn split_reasoning(
    response: &ResponseText,
    thinking: Option<&str>,
) -> (Option<String>, Option<String>) {
    if let Some(field) = thinking.filter(|text| !text.is_empty()) {
        let content = match response {
            ResponseText::Absent => Some(String::new()),
            ResponseText::Null => None,
            ResponseText::Text(text) => Some(text.clone()),
        };
        return (Some(field.to_owned()), content);
    }
    match response {
        ResponseText::Text(text) => parse_content_for_reasoning(Some(text.as_str())),
        ResponseText::Null => (None, None),
        ResponseText::Absent => (None, Some(String::new())),
    }
}

impl TryFrom<&[u8]> for GenerateReply {
    type Error = OllamaError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let value: Json =
            serde_json::from_slice(bytes).map_err(|source| OllamaError::Shape { source })?;
        match value.as_object().map(|fields| fields.get("message")) {
            Some(Some(Json::Object(_)) | None) => {}
            Some(_) => return Err(OllamaError::ReplyField { field: "message" }),
            None => return Err(OllamaError::ReplyField { field: "body" }),
        }
        serde_json::from_slice(bytes).map_err(|source| OllamaError::Shape { source })
    }
}

pub async fn transform_response(
    input: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
    ids: &dyn IdSource,
) -> Result<OllamaChatResponse, OllamaGenerateError> {
    let reply = GenerateReply::try_from(input.body)?;
    let (content, reasoning_content, tool_calls, finish_reason) = if matches!(input.request_data.get("format"), Some(Json::Text(format)) if format == JSON_FORMAT)
    {
        json_mode_message(&reply, ids)?
    } else {
        let (reasoning_content, content) =
            split_reasoning(&reply.response, reply.thinking.as_deref());
        (content, reasoning_content, None, STOP_FINISH_REASON)
    };
    let found_usage = token_usage(&reply, input.request_data, counter).await?;
    Ok(OllamaChatResponse {
        model: format!("{OLLAMA_PREFIX}{}", input.model),
        created: input.created,
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from(ASSISTANT_ROLE),
                content,
                reasoning_content,
                tool_calls,
            },
            finish_reason: String::from(finish_reason),
        }],
        usage: found_usage,
    })
}

type GeneratedMessage = (
    Option<String>,
    Option<String>,
    Option<Vec<ToolCall>>,
    &'static str,
);

// LiteLLM's json-mode branch: an empty response, a function call, plain JSON text, or the tag fallback
fn json_mode_message(
    reply: &GenerateReply,
    ids: &dyn IdSource,
) -> Result<GeneratedMessage, OllamaError> {
    let thinking = reply.thinking.clone().filter(|text| !text.is_empty());
    if reply.response.text().trim().is_empty() {
        return Ok((Some(String::new()), thinking, None, STOP_FINISH_REASON));
    }
    match serde_json::from_str::<Json>(reply.response.text()) {
        Ok(Json::Object(fields))
            if fields.contains_key(NAME_FIELD) && fields.contains_key(ARGUMENTS_FIELD) =>
        {
            let call = function_call(&fields, ids)?;
            Ok((None, thinking, Some(vec![call]), TOOL_CALLS_FINISH_REASON))
        }
        Ok(value) => {
            let content =
                serde_json::to_string(&value).map_err(|source| OllamaError::Shape { source })?;
            Ok((Some(content), thinking, None, STOP_FINISH_REASON))
        }
        Err(_error) => {
            let (reasoning_content, content) =
                split_reasoning(&reply.response, reply.thinking.as_deref());
            Ok((content, reasoning_content, None, STOP_FINISH_REASON))
        }
    }
}

fn function_call(fields: &Object, ids: &dyn IdSource) -> Result<ToolCall, OllamaError> {
    let name = match fields.get(NAME_FIELD) {
        Some(Json::Text(name)) => Some(name.clone()),
        Some(Json::Null) => None,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) | None => {
            return Err(OllamaError::ReplyField {
                field: "response.name",
            });
        }
    };
    let Some(value) = fields.get(ARGUMENTS_FIELD) else {
        return Err(OllamaError::ReplyField {
            field: "response.arguments",
        });
    };
    let serialized =
        serde_json::to_string(value).map_err(|source| OllamaError::Shape { source })?;
    Ok(ToolCall {
        id: format!("call_{}", ids.next_id()),
        kind: String::from(FUNCTION_TYPE),
        function: Function {
            name,
            arguments: serialized,
        },
    })
}

async fn token_usage(
    reply: &GenerateReply,
    request_data: &IndexMap<String, Json>,
    counter: &dyn TokenCounter,
) -> Result<Usage, OllamaError> {
    let prompt_tokens = match reply.prompt_eval_count {
        OptionalCount::Count(count) => Ok(count),
        OptionalCount::Absent => Ok(counter
            .count_text(prompt_text(request_data)?)
            .await
            .unwrap_or(0)),
        OptionalCount::Null => Err(OllamaError::ReplyField {
            field: "prompt_eval_count",
        }),
    }?;
    let counted_completion = message_char_count(reply.message.as_ref())?;
    let completion_tokens = match reply.eval_count {
        OptionalCount::Count(count) => Ok(count),
        OptionalCount::Absent => Ok(counted_completion),
        OptionalCount::Null => Err(OllamaError::ReplyField {
            field: "eval_count",
        }),
    }?;
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
    })
}

fn message_char_count(message: Option<&ReplyMessage>) -> Result<u64, OllamaError> {
    match message.map(|entry| &entry.content) {
        Some(ResponseText::Text(text)) => {
            Ok(u64::try_from(text.chars().count()).unwrap_or(u64::MAX))
        }
        Some(ResponseText::Absent) | None => Ok(0),
        Some(ResponseText::Null) => Err(OllamaError::ReplyField {
            field: "message.content",
        }),
    }
}

fn prompt_text(request_data: &IndexMap<String, Json>) -> Result<&str, OllamaError> {
    match request_data.get("prompt") {
        Some(Json::Text(text)) => Ok(text),
        Some(Json::Null) | None => Ok(""),
        Some(_) => Err(OllamaError::Param { field: "prompt" }),
    }
}
