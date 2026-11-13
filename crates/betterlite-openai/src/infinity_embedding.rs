use std::str;

use betterlite_config::sections::embedding_host_settings::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;
use serde_json::error::Category;

use crate::embedding::EmbeddingRecord;
use crate::openai_base_config::{Headers, Object};

use self::error::InfinityEmbeddingError;

pub mod error;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["encoding_format", "modality", "dimensions"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfinityEmbeddingResponse {
    pub model: Option<String>,
    pub data: Option<Vec<EmbeddingRecord>>,
    pub object: Option<String>,
    pub usage: InfinityUsage,
}

// the two counters are read independently and never reconciled
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InfinityUsage {
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

pub fn get_complete_url(api_base: Option<&str>) -> Result<String, InfinityEmbeddingError> {
    let base = api_base.ok_or(InfinityEmbeddingError::MissingApiBase)?;
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/embeddings") {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("{trimmed}/embeddings"))
    }
}

// only a missing key reads the environment, so an explicit empty key stays empty
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Headers {
    let key = api_key
        .or(settings.infinity_api_key.as_ref())
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

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for name in ["encoding_format", "modality"] {
        if let Some(value) = non_default_params.get(name) {
            optional_params.insert(name.to_owned(), value.clone());
        }
    }
    if let Some(dimensions) = non_default_params.get("dimensions") {
        optional_params.insert(String::from("output_dimension"), dimensions.clone());
    }
}

// the options go last, so they can replace both input and model
#[must_use]
pub fn transform_embedding_request(model: &str, input: &Value, optional_params: &Object) -> Object {
    let mut body = Object::from_iter([
        (String::from("input"), input.clone()),
        (String::from("model"), Value::String(model.to_owned())),
    ]);
    body.extend(optional_params.clone());
    body
}

pub fn decode_infinity_embedding(
    raw: &[u8],
    status_code: u16,
) -> Result<InfinityEmbeddingResponse, InfinityEmbeddingError> {
    let wire = serde_json::from_slice::<Wire>(raw).map_err(|source| match source.classify() {
        Category::Data => InfinityEmbeddingError::Shape { source },
        Category::Io | Category::Syntax | Category::Eof => InfinityEmbeddingError::Json {
            status_code,
            message: str::from_utf8(raw).map_or_else(|_| source.to_string(), str::to_owned),
            source,
        },
    })?;
    let usage = wire.usage.unwrap_or_default();
    Ok(InfinityEmbeddingResponse {
        model: wire.model,
        data: wire.data,
        object: wire.object,
        usage: InfinityUsage {
            prompt_tokens: usage.prompt_tokens.unwrap_or_default(),
            total_tokens: usage.total_tokens.unwrap_or_default(),
        },
    })
}

mod tests;
