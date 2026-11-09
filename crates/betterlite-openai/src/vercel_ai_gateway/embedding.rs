use std::str;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::embedding::{EmbeddingError, EmbeddingInput, EmbeddingResponse};
use crate::openai_base_config::{Headers, Object};
use crate::vercel_ai_gateway::{DEFAULT_API_BASE, VercelAiGatewayEnvironment};

const MODEL_PREFIX: &str = "vercel_ai_gateway/";

// LiteLLM advertises exactly these four embedding parameters and drops every other request key
pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["timeout", "dimensions", "encoding_format", "user"];

#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut merged = Headers::from([(
        String::from("Content-Type"),
        String::from("application/json"),
    )]);
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        merged.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    // the caller's headers win on an exact key, and a differently cased one is a second entry
    merged.extend(headers.clone());
    merged
}

#[must_use]
pub fn get_complete_url(
    api_base: Option<&str>,
    environment: &VercelAiGatewayEnvironment,
) -> String {
    let base = api_base.filter(|base| !base.is_empty()).map_or_else(
        || {
            environment
                .vercel_ai_gateway_api_base
                .as_deref()
                .filter(|base| !base.is_empty())
                .unwrap_or(DEFAULT_API_BASE)
        },
        |base| base.trim_end_matches('/'),
    );
    format!("{base}/embeddings")
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

pub fn decode_embedding_response(raw: &[u8]) -> Result<EmbeddingResponse, EmbeddingError> {
    serde_json::from_slice::<EmbeddingResponse>(raw)
        .map_err(|source| EmbeddingError::Decode { source })
}

pub fn transform_embedding_response(
    raw: &[u8],
    post_call: &mut dyn FnMut(&str),
) -> Result<EmbeddingResponse, EmbeddingError> {
    if let Ok(text) = str::from_utf8(raw) {
        post_call(text);
    }
    decode_embedding_response(raw)
}
