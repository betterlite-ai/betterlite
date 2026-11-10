use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::slice;
use std::str;

use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use thiserror::Error;

use super::error::{BytezConfigError, BytezError};
use super::messages::{RawMessage, get_tokens_from_messages, normalize_messages};

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
    let fields: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(input.body)?;
    if let Some(error) = field::<Json>(&fields, "error")? {
        let message =
            serde_json::to_string(&error).map_err(|source| BytezReplyError::Encode { source })?;
        return Err(BytezReplyError::Api(BytezError {
            status_code: input.status,
            message,
        }));
    }
    let headers = decoded_headers(input.headers);
    let output = field::<RawMessage>(&fields, "output")?.ok_or(BytezReplyError::MissingOutput)?;
    let content = output_text(&output)?.to_owned();
    let prompt = normalize_messages(input.messages)?;
    let prompt_tokens = get_tokens_from_messages(&prompt)?;
    let completion = normalize_messages(slice::from_ref(&output))?;
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
        provider_specific_fields: rate_limit_fields(&headers),
        additional_headers: response_headers(&headers),
    })
}

// the root must be an object: LiteLLM calls json.get("error") on it, so a list root fails there
fn field<T>(
    fields: &BTreeMap<String, Box<RawValue>>,
    name: &str,
) -> Result<Option<T>, serde_json::Error>
where
    T: DeserializeOwned,
{
    let Some(raw) = fields.get(name) else {
        return Ok(None);
    };
    serde_json::from_str(raw.get())
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

fn rate_limit_fields(headers: &[(String, String)]) -> RateLimitFields {
    RateLimitFields {
        ratelimit_limit: header_text(headers, "ratelimit-limit"),
        ratelimit_remaining: header_text(headers, "ratelimit-remaining"),
        ratelimit_reset: header_text(headers, "ratelimit-reset"),
        inference_meter: header_text(headers, "inference-meter"),
        inference_time: header_text(headers, "inference-time"),
    }
}

// httpx joins repeated header values with ", "; a header that is present but empty reads as ""
fn header_text(headers: &[(String, String)], name: &str) -> Option<String> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|(key, _)| key.as_str() == name)
        .map(|(_, value)| value.as_str())
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}

// httpx decodes every header value as UTF-8, or as Latin-1 when any value is not UTF-8
fn decoded_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    let utf8 = headers
        .values()
        .all(|value| str::from_utf8(value.as_bytes()).is_ok());
    headers
        .iter()
        .map(|(name, value)| {
            let bytes = value.as_bytes();
            let text = str::from_utf8(bytes).ok().filter(|_| utf8).map_or_else(
                || bytes.iter().map(|byte| char::from(*byte)).collect(),
                str::to_owned,
            );
            (name.as_str().to_owned(), text)
        })
        .collect()
}

fn response_headers(headers: &[(String, String)]) -> BTreeMap<String, String> {
    let mut mapped: BTreeMap<String, String> = BTreeMap::new();
    for (name, text) in headers {
        match mapped.entry(name.clone()) {
            Entry::Occupied(mut entry) => {
                let joined = entry.get_mut();
                joined.push_str(", ");
                joined.push_str(text);
            }
            Entry::Vacant(entry) => {
                entry.insert(text.clone());
            }
        }
    }
    mapped
}
