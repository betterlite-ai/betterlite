use std::str;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;
use serde_json::error::Category;

use crate::embedding::EmbeddingRecord;
use crate::openai_base_config::{Headers, Object};

use self::error::{SambaNovaEmbeddingError, SambaNovaError};

pub mod error;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SambaNovaEmbeddingResponse {
    pub model: Option<String>,
    pub data: Option<Vec<EmbeddingRecord>>,
    pub object: Option<String>,
    pub usage: SambaNovaUsage,
}

// the two counters are read independently and each falls back to zero on its own
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SambaNovaUsage {
    pub prompt_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Deserialize)]
struct Wire {
    model: Option<String>,
    data: Option<Vec<EmbeddingRecord>>,
    object: Option<String>,
    usage: Option<WireUsage>,
}

#[derive(Default, Deserialize)]
struct WireUsage {
    prompt_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

pub fn get_complete_url(api_base: Option<&str>) -> Result<String, SambaNovaEmbeddingError> {
    let base = api_base.ok_or(SambaNovaEmbeddingError::MissingApiBase)?;
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/embeddings") {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("{trimmed}/embeddings"))
    }
}

// the explicit key wins even when it is empty, and an absent key still writes the text Bearer None
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    env_api_key: Option<&SecretString>,
) -> Headers {
    let key = api_key
        .or(env_api_key)
        .map_or("None", |secret| secret.expose_secret());
    let mut merged = Headers::from([
        (String::from("Authorization"), format!("Bearer {key}")),
        (String::from("accept"), String::from("application/json")),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ]);
    merged.extend(headers.clone());
    merged
}

// the options go last, so an optional param named input or model replaces the one written before it
#[must_use]
pub fn transform_embedding_request(model: &str, input: &Value, optional_params: &Object) -> Object {
    let mut body = Object::from_iter([
        (String::from("input"), input.clone()),
        (String::from("model"), Value::String(model.to_owned())),
    ]);
    body.extend(optional_params.clone());
    body
}

pub fn decode_sambanova_embedding(
    raw: &[u8],
    status_code: u16,
    headers: &Headers,
) -> Result<SambaNovaEmbeddingResponse, SambaNovaEmbeddingError> {
    let wire = serde_json::from_slice::<Wire>(raw).map_err(|source| match source.classify() {
        Category::Data => SambaNovaEmbeddingError::Shape { source },
        Category::Io | Category::Syntax | Category::Eof => {
            SambaNovaEmbeddingError::Reply(SambaNovaError {
                status_code,
                message: str::from_utf8(raw).map_or_else(|_| source.to_string(), str::to_owned),
                headers: headers.clone(),
            })
        }
    })?;
    let usage = wire.usage.unwrap_or_default();
    Ok(SambaNovaEmbeddingResponse {
        model: wire.model,
        data: wire.data,
        object: wire.object,
        usage: SambaNovaUsage {
            prompt_tokens: usage.prompt_tokens.unwrap_or_default(),
            total_tokens: usage.total_tokens.unwrap_or_default(),
        },
    })
}

mod tests;
