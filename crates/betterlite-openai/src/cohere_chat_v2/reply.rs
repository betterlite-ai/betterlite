use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::cohere_streams::{CohereUsage, cohere_error};
use crate::error::ProviderError;

use self::citations::Annotation;
use self::tool_calls::ReplyToolCall;

pub mod citations;
pub mod tool_calls;

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error("the Cohere response is not valid JSON: {1}")]
    Json(u16, #[source] JsonError),
    #[error("the Cohere response is not a JSON object")]
    NotAMapping,
    #[error("the Cohere {0} field {1}")]
    Field(&'static str, &'static str),
    #[error("the Cohere tool arguments could not be written as JSON: {0}")]
    Encode(#[source] JsonError),
    #[error("the Cohere token usage exceeds the supported range")]
    UsageOverflow,
}

impl ReplyError {
    // the two stages LiteLLM tags with a status keep it; field access failures carry none
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Json(status, _) => Some(*status),
            Self::NotAMapping => Some(422),
            Self::Field(..) | Self::Encode(_) | Self::UsageOverflow => None,
        }
    }
}

impl From<ReplyError> for ProviderError {
    // get_error_class builds the shared Cohere error, which takes no headers at all
    fn from(error: ReplyError) -> Self {
        cohere_error(error.status_code().unwrap_or(500), error.to_string())
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssistantReply {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ReplyToolCall>>,
    pub annotations: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CohereV2Reply {
    pub id: String,
    pub created: u64,
    pub model: String,
    pub finish_reason: Option<String>,
    pub message: AssistantReply,
    pub usage: CohereUsage,
}

pub struct ReplyInput<'request> {
    pub body: &'request [u8],
    pub status: u16,
    pub model: &'request str,
    pub created: u64,
    pub id: String,
    pub finish_reason: Option<String>,
    pub message: AssistantReply,
    // LiteLLM mints a uuid for a tool call whose declared id is absent or null
    pub next_id: fn() -> String,
}

pub fn decode_cohere_chat_v2(input: ReplyInput<'_>) -> Result<CohereV2Reply, ReplyError> {
    let decoded: Json = serde_json::from_slice(input.body)
        .map_err(|source| ReplyError::Json(input.status, source))?;
    // CohereV2ChatResponse is a TypedDict: only keyword expansion of a non-mapping root fails here
    let Json::Object(response) = decoded else {
        return Err(ReplyError::NotAMapping);
    };
    let fields = match response.get("message") {
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(ReplyError::Field("message", "is not an object")),
        None => return Err(ReplyError::Field("message", "is missing")),
    };
    Ok(CohereV2Reply {
        id: input.id,
        created: input.created,
        model: input.model.to_owned(),
        finish_reason: input.finish_reason,
        message: reply_message(input.message, fields, input.next_id)?,
        usage: usage(&response)?,
    })
}

fn reply_message(
    mut reply: AssistantReply,
    fields: &Object,
    next_id: fn() -> String,
) -> Result<AssistantReply, ReplyError> {
    match fields.get("content") {
        None | Some(Json::Null) => {}
        Some(Json::List(parts)) => reply.content = Some(concatenate(parts)?),
        Some(_) => return Err(ReplyError::Field("message.content", "is not a list")),
    }
    let annotations = field_annotations(fields)?;
    if let Some(tools) = tool_calls::field_tool_calls(fields, next_id)? {
        return Ok(AssistantReply {
            content: None,
            tool_calls: Some(tools),
            annotations,
        });
    }
    if annotations.is_some() {
        reply.annotations = annotations;
    }
    Ok(reply)
}

fn concatenate(parts: &[Json]) -> Result<String, ReplyError> {
    let mut text = String::new();
    for part in parts {
        match part {
            Json::Null => {}
            Json::Object(fields) => match fields.get("text") {
                None | Some(Json::Null) => {}
                Some(Json::Text(value)) => text.push_str(value),
                Some(_) => {
                    return Err(ReplyError::Field("message.content.text", "is not a string"));
                }
            },
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                return Err(ReplyError::Field("message.content", "is not an object"));
            }
        }
    }
    Ok(text)
}

fn field_annotations(fields: &Object) -> Result<Option<Vec<Annotation>>, ReplyError> {
    match fields.get("citations").filter(|value| value.is_truthy()) {
        Some(Json::List(entries)) => {
            let found = citations::annotations(entries)?;
            Ok((!found.is_empty()).then_some(found))
        }
        Some(_) => Err(ReplyError::Field("message.citations", "is not a list")),
        None => Ok(None),
    }
}

fn usage(response: &Object) -> Result<CohereUsage, ReplyError> {
    let usage = match response.get("usage") {
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(ReplyError::Field("usage", "is not an object")),
        None => return Err(ReplyError::Field("usage", "is missing")),
    };
    let tokens = match usage.get("tokens") {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => return Err(ReplyError::Field("usage.tokens", "is not an object")),
    };
    let prompt_tokens = integer(
        tokens.and_then(|fields| fields.get("input_tokens")),
        "usage.tokens.input_tokens",
    )?;
    let completion_tokens = integer(
        tokens.and_then(|fields| fields.get("output_tokens")),
        "usage.tokens.output_tokens",
    )?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(ReplyError::UsageOverflow)?;
    Ok(CohereUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

pub(super) fn integer(value: Option<&Json>, field: &'static str) -> Result<i64, ReplyError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(ReplyError::Field(field, "is not an integer")),
        Some(_) => Err(ReplyError::Field(field, "is not an integer")),
    }
}
