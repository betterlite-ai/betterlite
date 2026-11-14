use std::str;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;

use self::error::HostedVllmEmbeddingError;
use crate::embedding::{EmbeddingInput, EmbeddingRecord, EmbeddingResponse, EmbeddingUsage};
use crate::openai_base_config::{Headers, Object};

pub mod error;
mod tests;

pub const MODEL_PREFIX: &str = "hosted_vllm/";
pub const PLACEHOLDER_KEY: &str = "fake-api-key";
const API_BASE_ENV: &str = "HOSTED_VLLM_API_BASE";
const API_KEY_ENV: &str = "HOSTED_VLLM_API_KEY";
const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";

// LiteLLM advertises exactly these four embedding parameters and drops every other request key
pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["timeout", "dimensions", "encoding_format", "user"];

#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> Headers {
    let key = api_key.map_or_else(
        || {
            environment
                .lookup(API_KEY_ENV)
                .filter(|key| !key.is_empty())
                .unwrap_or_else(|| String::from(PLACEHOLDER_KEY))
        },
        |key| key.expose_secret().to_owned(),
    );
    let mut merged =
        Headers::from([(String::from(CONTENT_TYPE), String::from("application/json"))]);
    if !key.is_empty() && key != PLACEHOLDER_KEY {
        merged.insert(String::from(AUTHORIZATION), format!("Bearer {key}"));
    }
    // the caller's headers win on an exact key, and a differently cased one is a second entry
    merged.extend(headers.clone());
    merged
}

pub fn get_complete_url(
    api_base: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> Result<String, HostedVllmEmbeddingError> {
    let base = api_base.map_or_else(
        || {
            environment
                .lookup(API_BASE_ENV)
                .filter(|base| !base.is_empty())
                .ok_or(HostedVllmEmbeddingError::MissingApiBase)
        },
        |base| Ok(String::from(base)),
    )?;
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/embeddings") {
        Ok(String::from(trimmed))
    } else {
        Ok(format!("{trimmed}/embeddings"))
    }
}

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

#[must_use]
pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Object {
    let mut body = Object::from_iter([
        (
            String::from("model"),
            Value::String(model.strip_prefix(MODEL_PREFIX).unwrap_or(model).to_owned()),
        ),
        (String::from("input"), input_value(input)),
    ]);
    body.extend(optional_params.clone());
    body
}

fn input_value(input: &EmbeddingInput) -> Value {
    match input {
        EmbeddingInput::Text(text) => Value::Array(vec![Value::String(text.clone())]),
        EmbeddingInput::Texts(texts) => {
            Value::Array(texts.iter().cloned().map(Value::String).collect())
        }
        EmbeddingInput::Tokens(tokens) => {
            Value::Array(tokens.iter().map(|token| Value::from(*token)).collect())
        }
        EmbeddingInput::TokenLists(lists) => Value::Array(
            lists
                .iter()
                .map(|list| Value::Array(list.iter().map(|token| Value::from(*token)).collect()))
                .collect(),
        ),
    }
}

#[derive(Deserialize)]
struct RawReply {
    model: String,
    #[serde(default)]
    object: Option<String>,
    data: Vec<EmbeddingRecord>,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

pub fn transform_embedding_response(
    raw: &[u8],
    post_call: &mut dyn FnMut(&str),
) -> Result<EmbeddingResponse, HostedVllmEmbeddingError> {
    // LiteLLM logs the answer text before it parses it
    if let Ok(text) = str::from_utf8(raw) {
        post_call(text);
    }
    let reply: RawReply =
        serde_json::from_slice(raw).map_err(|source| HostedVllmEmbeddingError::Shape { source })?;
    let mut usage = EmbeddingUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        completion_tokens_details: None,
        prompt_tokens_details: None,
        total_tokens: 0,
    };
    if let Some(given) = &reply.usage {
        usage.prompt_tokens = given.prompt_tokens;
        usage.completion_tokens = given.completion_tokens;
        usage.total_tokens = given.total_tokens;
    }
    Ok(EmbeddingResponse {
        object: reply.object.unwrap_or_else(|| String::from("list")),
        data: reply.data,
        model: reply.model,
        usage,
    })
}

#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: &Headers,
) -> HostedVllmEmbeddingError {
    HostedVllmEmbeddingError::Provider {
        message: message.to_owned(),
        status_code,
        headers: headers.clone(),
    }
}
