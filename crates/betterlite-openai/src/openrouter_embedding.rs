mod tests;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Number, error::Category};
use thiserror::Error;

use crate::embedding::{EmbeddingInput, EmbeddingResponse};
use crate::openai_base_config::Headers;

const DEFAULT_API_BASE: &str = "https://openrouter.ai/api/v1";
const DEFAULT_SITE_URL: &str = "https://litellm.ai";
const DEFAULT_APP_NAME: &str = "liteLLM";
const MODEL_PREFIX: &str = "openrouter/";

// LiteLLM advertises exactly these four embedding parameters and drops every other request key
pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["timeout", "dimensions", "encoding_format", "user"];

#[derive(Debug, Error)]
pub enum EmbeddingDecodeError {
    #[error("the OpenRouter embedding reply is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the OpenRouter embedding reply does not have the shape of an embeddings response")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
}

#[must_use]
pub fn validate_environment(
    headers: Headers,
    api_key: Option<&SecretString>,
    site_url: Option<&str>,
    app_name: Option<&str>,
) -> Headers {
    let site = site_url
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_SITE_URL);
    let app = app_name
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_APP_NAME);
    let mut merged = Headers::from([
        (String::from("HTTP-Referer"), site.to_owned()),
        (String::from("X-Title"), app.to_owned()),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ]);
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        merged.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    // a plain map overlay keeps a caller key that differs only in case from a default one
    merged.extend(headers);
    merged
}

#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .filter(|base| !base.is_empty())
        .map_or(DEFAULT_API_BASE, |base| base.trim_end_matches('/'));
    format!("{base}/embeddings")
}

#[must_use]
pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Object {
    let mut body = Object::new();
    body.insert(
        String::from("model"),
        Json::Text(model.strip_prefix(MODEL_PREFIX).unwrap_or(model).to_owned()),
    );
    body.insert(String::from("input"), input_json(input));
    for (name, value) in optional_params {
        body.insert(name.clone(), value.clone());
    }
    body
}

fn input_json(input: &EmbeddingInput) -> Json {
    match input {
        EmbeddingInput::Text(text) => Json::List(vec![Json::Text(text.clone())]),
        EmbeddingInput::Texts(texts) => Json::List(texts.iter().cloned().map(Json::Text).collect()),
        EmbeddingInput::Tokens(tokens) => Json::List(
            tokens
                .iter()
                .map(|token| Json::Number(Number::from(*token)))
                .collect(),
        ),
        EmbeddingInput::TokenLists(lists) => Json::List(
            lists
                .iter()
                .map(|list| {
                    Json::List(
                        list.iter()
                            .map(|token| Json::Number(Number::from(*token)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

pub fn decode_embedding_response(raw: &[u8]) -> Result<EmbeddingResponse, EmbeddingDecodeError> {
    serde_json::from_slice(raw).map_err(|source| {
        if source.classify() == Category::Data {
            EmbeddingDecodeError::Shape { source }
        } else {
            EmbeddingDecodeError::Json { source }
        }
    })
}

// the post-call log sees the raw text before the body is decoded, as the pinned method does
pub fn transform_embedding_response(
    raw: &[u8],
    post_call: &mut dyn FnMut(&str),
) -> Result<EmbeddingResponse, EmbeddingDecodeError> {
    if let Ok(text) = std::str::from_utf8(raw) {
        post_call(text);
    }
    decode_embedding_response(raw)
}
