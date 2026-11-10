use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::slice;

use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::error::{BytezConfigError, BytezError};
use super::messages::{RawMessage, adapt_messages_to_bytez_standard, get_tokens_from_messages};

// the pooled transport caps a reply at 16 MiB; a decoder handed more than that refuses it
pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BytezUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RateLimitFields {
    #[serde(rename = "ratelimit-limit")]
    pub ratelimit_limit: Option<String>,
    #[serde(rename = "ratelimit-remaining")]
    pub ratelimit_remaining: Option<String>,
    #[serde(rename = "ratelimit-reset")]
    pub ratelimit_reset: Option<String>,
    #[serde(rename = "inference-meter")]
    pub inference_meter: Option<String>,
    #[serde(rename = "inference-time")]
    pub inference_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BytezReply {
    pub created: i64,
    pub model: String,
    pub content: String,
    pub usage: BytezUsage,
    pub provider_specific_fields: RateLimitFields,
    pub additional_headers: BTreeMap<String, String>,
}

pub struct ReplyInput<'request> {
    pub body: &'request [u8],
    pub status: u16,
    pub model: &'request str,
    pub created: i64,
    pub messages: &'request [RawMessage],
    pub headers: &'request HeaderMap,
}

#[derive(Debug, Deserialize)]
struct ReplyBody {
    #[serde(default)]
    error: Option<Json>,
    #[serde(default)]
    output: Option<RawMessage>,
}

#[derive(Debug, Error)]
pub enum BytezReplyError {
    #[error(transparent)]
    Api(#[from] BytezError),
    #[error("the Bytez reply is {size} bytes, over the {MAX_REPLY_BYTES} byte limit")]
    TooLarge { size: usize },
    #[error("the Bytez reply is not usable JSON")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("the Bytez reply has no output")]
    MissingOutput,
    #[error("the Bytez output holds no text content")]
    OutputContent,
    #[error("the Bytez reply cannot be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Content(#[from] BytezConfigError),
    #[error("the Bytez token usage overflows")]
    UsageOverflow,
}

impl BytezReplyError {
    // a reply LiteLLM itself rejected carries its status; a decode failure the caller sees uncarried
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Api(error) => Some(error.status_code),
            Self::TooLarge { .. }
            | Self::Json { .. }
            | Self::MissingOutput
            | Self::OutputContent
            | Self::Encode { .. }
            | Self::Content(_)
            | Self::UsageOverflow => None,
        }
    }
}

pub fn decode_bytez_reply(input: &ReplyInput<'_>) -> Result<BytezReply, BytezReplyError> {
    if input.body.len() > MAX_REPLY_BYTES {
        return Err(BytezReplyError::TooLarge {
            size: input.body.len(),
        });
    }
    let body: ReplyBody = serde_json::from_slice(input.body)?;
    if let Some(error) = body
        .error
        .as_ref()
        .filter(|value| !matches!(value, Json::Null))
    {
        let message =
            serde_json::to_string(error).map_err(|source| BytezReplyError::Encode { source })?;
        return Err(BytezReplyError::Api(BytezError {
            status_code: input.status,
            message,
        }));
    }
    let output = body.output.ok_or(BytezReplyError::MissingOutput)?;
    let content = output_text(&output)?.to_owned();
    let prompt = adapt_messages_to_bytez_standard(input.messages)?;
    let prompt_tokens = get_tokens_from_messages(&prompt)?;
    let completion = adapt_messages_to_bytez_standard(slice::from_ref(&output))?;
    let completion_tokens = get_tokens_from_messages(&completion)?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(BytezReplyError::UsageOverflow)?;
    Ok(BytezReply {
        created: input.created,
        model: input.model.to_owned(),
        content,
        usage: BytezUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        },
        provider_specific_fields: rate_limit_fields(input.headers),
        additional_headers: response_headers(input.headers),
    })
}

// LiteLLM reads output.content[0].text straight out of the reply dict
fn output_text(output: &RawMessage) -> Result<&str, BytezReplyError> {
    let Some(Json::List(items)) = output.content.as_ref() else {
        return Err(BytezReplyError::OutputContent);
    };
    let Some(Json::Object(first)) = items.first() else {
        return Err(BytezReplyError::OutputContent);
    };
    match first.get("text") {
        Some(Json::Text(text)) => Ok(text),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BytezReplyError::OutputContent)
        }
    }
}

fn rate_limit_fields(headers: &HeaderMap) -> RateLimitFields {
    RateLimitFields {
        ratelimit_limit: header_text(headers, "ratelimit-limit"),
        ratelimit_remaining: header_text(headers, "ratelimit-remaining"),
        ratelimit_reset: header_text(headers, "ratelimit-reset"),
        inference_meter: header_text(headers, "inference-meter"),
        inference_time: header_text(headers, "inference-time"),
    }
}

// httpx joins repeated header values with ", " and treats names case-insensitively
fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    let joined = headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(", ");
    (!joined.is_empty()).then_some(joined)
}

fn response_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut mapped: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let Ok(text) = value.to_str() else {
            continue;
        };
        match mapped.entry(name.as_str().to_owned()) {
            Entry::Occupied(mut entry) => {
                let joined = entry.get_mut();
                joined.push_str(", ");
                joined.push_str(text);
            }
            Entry::Vacant(entry) => {
                entry.insert(text.to_owned());
            }
        }
    }
    mapped
}
