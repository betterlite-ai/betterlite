use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::cohere_streams::CohereUsage;

use self::citations::Annotation;
use self::tool_calls::ReplyToolCall;

pub mod citations;
pub mod tool_calls;

#[derive(Debug, Error)]
pub enum CohereV2Error {
    #[error("the Cohere response is not valid JSON: {source}")]
    Json {
        status: u16,
        #[source]
        source: JsonError,
    },
    #[error("the Cohere response is not a JSON object")]
    NotAMapping,
    #[error("the Cohere response has no {field} field")]
    Missing { field: &'static str },
    #[error("the Cohere {field} field is not an object")]
    NotAnObject { field: &'static str },
    #[error("the Cohere {field} field is not a list")]
    NotAList { field: &'static str },
    #[error("the Cohere {field} field is not an integer")]
    NotAnInteger { field: &'static str },
    #[error("the Cohere {field} field is not a string")]
    NotText { field: &'static str },
    #[error("the Cohere tool arguments could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: JsonError,
    },
    #[error("the Cohere token usage exceeds the supported range")]
    UsageOverflow,
}

impl CohereV2Error {
    // the two stages LiteLLM tags with a status keep it; field access failures carry none
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Json { status, .. } => Some(*status),
            Self::NotAMapping => Some(422),
            Self::Missing { .. }
            | Self::NotAnObject { .. }
            | Self::NotAList { .. }
            | Self::NotAnInteger { .. }
            | Self::NotText { .. }
            | Self::Encode { .. }
            | Self::UsageOverflow => None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssistantReply {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ReplyToolCall>>,
    pub annotations: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyTemplate {
    pub id: String,
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub message: AssistantReply,
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
    pub template: ReplyTemplate,
}

pub fn decode_cohere_chat_v2(input: ReplyInput<'_>) -> Result<CohereV2Reply, CohereV2Error> {
    let decoded: Json =
        serde_json::from_slice(input.body).map_err(|source| CohereV2Error::Json {
            status: input.status,
            source,
        })?;
    // CohereV2ChatResponse is a TypedDict: only keyword expansion of a non-mapping root fails here
    let Json::Object(response) = decoded else {
        return Err(CohereV2Error::NotAMapping);
    };
    let fields = match response.get("message") {
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(CohereV2Error::NotAnObject { field: "message" }),
        None => return Err(CohereV2Error::Missing { field: "message" }),
    };
    let ReplyTemplate {
        id,
        finish_reason,
        message,
    } = input.template;
    Ok(CohereV2Reply {
        id,
        created: input.created,
        model: input.model.to_owned(),
        finish_reason,
        message: reply_message(message, fields)?,
        usage: usage(&response)?,
    })
}

fn reply_message(
    mut reply: AssistantReply,
    fields: &Object,
) -> Result<AssistantReply, CohereV2Error> {
    match fields.get("content") {
        None | Some(Json::Null) => {}
        Some(Json::List(parts)) => reply.content = Some(concatenate(parts)?),
        Some(_) => {
            return Err(CohereV2Error::NotAList {
                field: "message.content",
            });
        }
    }
    let annotations = field_annotations(fields)?;
    let Some(tools) = tool_calls::field_tool_calls(fields)? else {
        if annotations.is_some() {
            reply.annotations = annotations;
        }
        return Ok(reply);
    };
    Ok(AssistantReply {
        content: None,
        tool_calls: Some(tools),
        annotations,
    })
}

fn concatenate(parts: &[Json]) -> Result<String, CohereV2Error> {
    let mut text = String::new();
    for part in parts {
        match part {
            Json::Null => {}
            Json::Object(fields) => match fields.get("text") {
                None | Some(Json::Null) => {}
                Some(Json::Text(value)) => text.push_str(value),
                Some(_) => {
                    return Err(CohereV2Error::NotText {
                        field: "message.content.text",
                    });
                }
            },
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                return Err(CohereV2Error::NotAnObject {
                    field: "message.content",
                });
            }
        }
    }
    Ok(text)
}

fn field_annotations(fields: &Object) -> Result<Option<Vec<Annotation>>, CohereV2Error> {
    match fields.get("citations").filter(|value| value.is_truthy()) {
        Some(Json::List(entries)) => {
            let found = citations::annotations(entries)?;
            Ok((!found.is_empty()).then_some(found))
        }
        Some(_) => Err(CohereV2Error::NotAList {
            field: "message.citations",
        }),
        None => Ok(None),
    }
}

fn usage(response: &Object) -> Result<CohereUsage, CohereV2Error> {
    let usage = match response.get("usage") {
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(CohereV2Error::NotAnObject { field: "usage" }),
        None => return Err(CohereV2Error::Missing { field: "usage" }),
    };
    let tokens = match usage.get("tokens") {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => {
            return Err(CohereV2Error::NotAnObject {
                field: "usage.tokens",
            });
        }
    };
    let prompt_tokens = integer(
        tokens.and_then(|fields| fields.get("input_tokens")),
        "usage.tokens.input_tokens",
    )?;
    let completion_tokens = integer(
        tokens.and_then(|fields| fields.get("output_tokens")),
        "usage.tokens.output_tokens",
    )?;
    Ok(CohereUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens
            .checked_add(completion_tokens)
            .ok_or(CohereV2Error::UsageOverflow)?,
    })
}

pub(super) fn integer(value: Option<&Json>, field: &'static str) -> Result<i64, CohereV2Error> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(CohereV2Error::NotAnInteger { field }),
        Some(_) => Err(CohereV2Error::NotAnInteger { field }),
    }
}
