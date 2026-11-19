use std::fmt;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::embedding::{EmbeddingInput, EmbeddingResponse};
use crate::gigachat_chat::MODEL_PREFIX;
use crate::gigachat_chat::config::GigachatBearer;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::get_api_base;
use crate::openai_base_config::{Headers, Object};

mod tests;

const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
// LiteLLM's generic status for an exception it cannot classify
const FAILURE_STATUS: u16 = 500;

// LiteLLM declares no embedding option for GigaChat and maps every other parameter away
pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];

#[derive(Debug, Error)]
#[error("{message}")]
pub struct GigaChatEmbeddingError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
    #[source]
    pub source: Option<serde_json::Error>,
}

impl From<GigaChatError> for GigaChatEmbeddingError {
    fn from(error: GigaChatError) -> Self {
        let GigaChatError {
            status_code,
            message,
            headers,
            source,
        } = error;
        Self {
            status_code,
            message,
            headers,
            source,
        }
    }
}

#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: &Headers,
) -> GigaChatEmbeddingError {
    GigaChatEmbeddingError {
        status_code,
        message: message.to_owned(),
        headers: Some(headers.clone()),
        source: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum EmbeddingBodyInput {
    Texts(Vec<String>),
    Tokens(Vec<u32>),
    TokenLists(Vec<Vec<u32>>),
}

impl From<&EmbeddingInput> for EmbeddingBodyInput {
    fn from(input: &EmbeddingInput) -> Self {
        match input {
            EmbeddingInput::Text(text) => Self::Texts(vec![text.clone()]),
            EmbeddingInput::Texts(texts) => Self::Texts(texts.clone()),
            EmbeddingInput::Tokens(tokens) => Self::Tokens(tokens.clone()),
            EmbeddingInput::TokenLists(lists) => Self::TokenLists(lists.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingRequestBody {
    model: String,
    input: EmbeddingBodyInput,
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingRequest {
    pub url: String,
    pub headers: Headers,
    pub body: EmbeddingRequestBody,
}

impl fmt::Debug for EmbeddingRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddingRequest")
            .field("url", &self.url)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body", &self.body)
            .finish()
    }
}

pub struct EmbeddingRequestInput<'request> {
    pub model: &'request str,
    pub input: &'request EmbeddingInput,
    pub api_base: Option<&'request str>,
    pub env_base: Option<&'request str>,
    pub credentials: Option<&'request SecretString>,
    pub caller_headers: &'request Headers,
}

pub async fn embedding_request(
    bearer: &dyn GigachatBearer,
    input: &EmbeddingRequestInput<'_>,
) -> Result<EmbeddingRequest, GigaChatEmbeddingError> {
    let credentials = input
        .credentials
        .filter(|value| !value.expose_secret().is_empty());
    let token = bearer.access_token(credentials).await?;
    let mut headers = Headers::from([
        (String::from(CONTENT_TYPE), String::from("application/json")),
        (
            String::from(AUTHORIZATION),
            format!("Bearer {}", token.expose_secret()),
        ),
    ]);
    headers.extend(input.caller_headers.clone());
    Ok(EmbeddingRequest {
        url: format!(
            "{}/embeddings",
            get_api_base(input.api_base, input.env_base)
        ),
        headers,
        body: EmbeddingRequestBody {
            model: input
                .model
                .strip_prefix(MODEL_PREFIX)
                .unwrap_or(input.model)
                .to_owned(),
            input: EmbeddingBodyInput::from(input.input),
        },
    })
}

pub fn transform_response(body: &[u8]) -> Result<EmbeddingResponse, GigaChatEmbeddingError> {
    let mut fields = serde_json::from_slice::<Object>(body).map_err(reply_error)?;
    let total = rewrite_item_usage(&mut fields)?;
    fields.insert(String::from("object"), Value::String(String::from("list")));
    fields.insert(String::from("usage"), Value::Object(usage_object(total)));
    if !fields.contains_key("data") {
        fields.insert(String::from("data"), Value::Array(Vec::new()));
    }
    serde_json::from_value::<EmbeddingResponse>(Value::Object(fields)).map_err(reply_error)
}

// LiteLLM rebuilds the response usage from the per-item prompt_tokens and drops the item usage
fn rewrite_item_usage(fields: &mut Object) -> Result<u64, GigaChatEmbeddingError> {
    let Some(data) = fields.get_mut("data") else {
        return Ok(0);
    };
    let Value::Array(items) = data else {
        // any other data fails the response decode below, where the field must be a list
        return Ok(0);
    };
    let mut total = 0_u64;
    for item in items {
        let Some(entry) = item.as_object_mut() else {
            return Err(invalid_reply("an embedding item is not an object"));
        };
        let Some(detached) = entry.remove("usage") else {
            continue;
        };
        let Some(counts) = detached.as_object() else {
            return Err(invalid_reply("an embedding item usage is not an object"));
        };
        let Some(reported) = counts.get("prompt_tokens") else {
            continue;
        };
        let Some(count) = reported.as_u64() else {
            return Err(invalid_reply(
                "an embedding item prompt_tokens is not a non-negative integer",
            ));
        };
        total = total.saturating_add(count);
    }
    Ok(total)
}

fn usage_object(total: u64) -> Object {
    let capped = u32::try_from(total).unwrap_or(u32::MAX);
    Object::from_iter([
        (String::from("prompt_tokens"), Value::from(capped)),
        (String::from("total_tokens"), Value::from(capped)),
    ])
}

fn reply_error(source: serde_json::Error) -> GigaChatEmbeddingError {
    GigaChatEmbeddingError {
        status_code: FAILURE_STATUS,
        message: format!("the GigaChat embedding reply cannot be read: {source}"),
        headers: None,
        source: Some(source),
    }
}

fn invalid_reply(reason: &str) -> GigaChatEmbeddingError {
    GigaChatEmbeddingError {
        status_code: FAILURE_STATUS,
        message: format!("the GigaChat embedding reply cannot be read: {reason}"),
        headers: None,
        source: None,
    }
}
