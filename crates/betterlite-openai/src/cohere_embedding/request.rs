use std::slice;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::base64_string::is_base64_encoded;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::{CohereEmbeddingError, DEFAULT_COMPLETE_URL, DEFAULT_EMBEDDING_INPUT_TYPE};
use crate::embedding::EmbeddingInput;
use crate::openai_base_config::Headers;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CohereEmbeddingRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    pub input_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_types: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dimension: Option<i64>,
    #[serde(flatten)]
    pub extra: Object,
}

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        match name.as_str() {
            "encoding_format" => {
                let types = if let Json::List(items) = value {
                    items.clone()
                } else {
                    vec![value.clone()]
                };
                optional_params.insert(String::from("embedding_types"), Json::List(types));
            }
            "dimensions" => {
                optional_params.insert(String::from("output_dimension"), value.clone());
            }
            _ => {}
        }
    }
    optional_params
}

#[must_use]
pub fn validate_environment(headers: Headers, api_key: Option<&SecretString>) -> Headers {
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
    merged.extend(headers);
    merged
}

#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    match api_base {
        Some(base) if !base.is_empty() => base.to_owned(),
        Some(_) | None => String::from(DEFAULT_COMPLETE_URL),
    }
}

// _transform_request: the last input alone decides image versus text, and Bedrock callers reach it directly
pub fn transform_request(
    model: &str,
    input: &[String],
    inference_params: &Object,
) -> Result<CohereEmbeddingRequest, CohereEmbeddingError> {
    let image = input.last().is_some_and(|text| is_base64_encoded(text));
    let built = CohereEmbeddingRequest {
        model: model.to_owned(),
        texts: (!image).then_some(input.to_vec()),
        images: image.then_some(input.to_vec()),
        input_type: String::from(if image {
            "image"
        } else {
            DEFAULT_EMBEDDING_INPUT_TYPE
        }),
        truncate: None,
        embedding_types: None,
        output_dimension: None,
        extra: Object::new(),
    };
    let mut fields: Object = serde_json::from_value(serde_json::to_value(built)?)?;
    fields.extend(
        inference_params
            .iter()
            .filter(|(_, value)| **value != Json::Null)
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    Ok(serde_json::from_value(serde_json::to_value(fields)?)?)
}

pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Result<CohereEmbeddingRequest, CohereEmbeddingError> {
    let texts: &[String] = match input {
        EmbeddingInput::Text(text) => slice::from_ref(text),
        EmbeddingInput::Texts(texts) if texts.is_empty() => {
            return Err(CohereEmbeddingError::EmptyInput);
        }
        EmbeddingInput::Texts(texts) => texts,
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            return Err(CohereEmbeddingError::InputNotText);
        }
    };
    transform_request(model, texts, optional_params)
}
