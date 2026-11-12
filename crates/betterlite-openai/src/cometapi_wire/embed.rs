use std::str;

use betterlite_config::sections::cometapi_embedding_credentials::Settings;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

use crate::openai_base_config::Headers;

use super::error::CometApiError;
use super::{DEFAULT_API_BASE, EMBEDDING_ENDPOINT};

pub const MAX_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;
pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["dimensions", "encoding_format", "user"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CometEmbeddingReply {
    pub model: Option<Json>,
    pub data: Option<Json>,
    pub object: Option<Json>,
    pub usage: CometEmbeddingUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CometEmbeddingUsage {
    pub prompt_tokens: i64,
    pub total_tokens: i64,
}

pub struct EmbeddingReplyInput<'body> {
    pub body: &'body [u8],
    pub status: u16,
    pub headers: &'body Headers,
}

#[derive(Debug, Error)]
pub enum EmbeddingReplyError {
    #[error(transparent)]
    Api(#[from] CometApiError),
    #[error("the CometAPI embedding reply is {size} bytes, over the {MAX_REPLY_BYTES} byte limit")]
    TooLarge { size: usize },
    #[error("the CometAPI embedding reply field {field} has an unexpected shape")]
    FieldShape { field: &'static str },
}

// an absent base takes the default, while any given one is trimmed and still gains /embeddings
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base.map_or(DEFAULT_API_BASE, |base| base.trim_end_matches('/'));
    format!("{base}/{EMBEDDING_ENDPOINT}")
}

// an absent caller key falls back to COMETAPI_KEY, and a missing one still writes Bearer None
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    environment: &Settings,
) -> Headers {
    let key = api_key.or(environment.api_key.as_ref());
    let authorization = key.map_or_else(
        || String::from("Bearer None"),
        |secret| format!("Bearer {}", secret.expose_secret()),
    );
    let mut merged = Headers::from([
        (String::from("Authorization"), authorization),
        (String::from("accept"), String::from("application/json")),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ]);
    merged.extend(headers.clone());
    merged
}

// the embedding adapter copies only the three declared options and honours neither drop_params nor the rest
pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// options sit over input and model, so an option with either name wins exactly as the dict merge does
#[must_use]
pub fn transform_embedding_request(model: &str, input: &Json, optional_params: &Object) -> Object {
    let mut body = Object::from([
        (String::from("input"), input.clone()),
        (String::from("model"), Json::Text(model.to_owned())),
    ]);
    body.extend(optional_params.clone());
    body
}

// model, data and object are copied with a nullable get; a missing usage contributes zero counters
pub fn decode_comet_embedding_reply(
    input: &EmbeddingReplyInput<'_>,
) -> Result<CometEmbeddingReply, EmbeddingReplyError> {
    if input.body.len() > MAX_REPLY_BYTES {
        return Err(EmbeddingReplyError::TooLarge {
            size: input.body.len(),
        });
    }
    let parsed = match serde_json::from_slice::<Json>(input.body) {
        Ok(parsed) => parsed,
        Err(_source) => {
            return Err(EmbeddingReplyError::Api(CometApiError {
                message: body_text(input.body),
                status_code: Some(input.status),
                headers: input.headers.clone(),
            }));
        }
    };
    let Json::Object(fields) = parsed else {
        return Err(EmbeddingReplyError::FieldShape { field: "body" });
    };
    Ok(CometEmbeddingReply {
        model: nullable(&fields, "model"),
        data: nullable(&fields, "data"),
        object: nullable(&fields, "object"),
        usage: usage_of(&fields)?,
    })
}

fn nullable(fields: &Object, name: &str) -> Option<Json> {
    match fields.get(name) {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

fn usage_of(fields: &Object) -> Result<CometEmbeddingUsage, EmbeddingReplyError> {
    let Some(usage) = fields.get("usage") else {
        return Ok(CometEmbeddingUsage {
            prompt_tokens: 0,
            total_tokens: 0,
        });
    };
    let Json::Object(usage_fields) = usage else {
        return Err(EmbeddingReplyError::FieldShape { field: "usage" });
    };
    Ok(CometEmbeddingUsage {
        prompt_tokens: counter(usage_fields, "prompt_tokens")?,
        total_tokens: counter(usage_fields, "total_tokens")?,
    })
}

fn counter(usage: &Object, name: &'static str) -> Result<i64, EmbeddingReplyError> {
    match usage.get(name) {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(EmbeddingReplyError::FieldShape { field: name }),
        Some(_) => Err(EmbeddingReplyError::FieldShape { field: name }),
    }
}

fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_source| String::from("the CometAPI embedding reply is not valid JSON"),
        str::to_owned,
    )
}
