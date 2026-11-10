use std::slice;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::base64_string::is_base64_encoded;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::{CohereEmbeddingError, DEFAULT_COMPLETE_URL};
use crate::embedding::EmbeddingInput;
use crate::openai_base_config::Headers;

// COHERE_EMBEDDING_INPUT_TYPES: the closed set the request builder and an overlay may write
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CohereInputType {
    SearchDocument,
    SearchQuery,
    Classification,
    Clustering,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CohereTruncate {
    None,
    Start,
    End,
}

// CohereEmbeddingRequestWithModel: model plus every request field optional_params can overlay
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CohereEmbeddingRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    pub input_type: CohereInputType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<CohereTruncate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_types: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dimension: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct InferenceParams {
    model: Option<String>,
    texts: Option<Vec<String>>,
    images: Option<Vec<String>>,
    input_type: Option<CohereInputType>,
    truncate: Option<CohereTruncate>,
    embedding_types: Option<Vec<Json>>,
    output_dimension: Option<i64>,
}

// map_openai_params: encoding_format stays a list, every other value becomes a one-entry list
#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        if name == "encoding_format" {
            let types = if let Json::List(items) = value {
                items.clone()
            } else {
                vec![value.clone()]
            };
            optional_params.insert(String::from("embedding_types"), Json::List(types));
        } else if name == "dimensions" {
            optional_params.insert(String::from("output_dimension"), value.clone());
        }
    }
    optional_params
}

// validate_environment: defaults first, then the caller's exact header spellings on top
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

// get_complete_url: a non-empty supplied base is the whole URL, never a base plus a suffix
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
    let mut request = CohereEmbeddingRequest {
        model: model.to_owned(),
        texts: (!image).then_some(input.to_vec()),
        images: image.then_some(input.to_vec()),
        input_type: if image {
            CohereInputType::Image
        } else {
            CohereInputType::SearchDocument
        },
        truncate: None,
        embedding_types: None,
        output_dimension: None,
    };
    let value = serde_json::to_value(inference_params)
        .map_err(|source| CohereEmbeddingError::Json { source })?;
    let params: InferenceParams =
        serde_json::from_value(value).map_err(|source| CohereEmbeddingError::Json { source })?;
    if let Some(model) = params.model {
        request.model = model;
    }
    if let Some(texts) = params.texts {
        request.texts = Some(texts);
    }
    if let Some(images) = params.images {
        request.images = Some(images);
    }
    if let Some(input_type) = params.input_type {
        request.input_type = input_type;
    }
    if let Some(truncate) = params.truncate {
        request.truncate = Some(truncate);
    }
    if let Some(embedding_types) = params.embedding_types {
        request.embedding_types = Some(embedding_types);
    }
    if let Some(output_dimension) = params.output_dimension {
        request.output_dimension = Some(output_dimension);
    }
    Ok(request)
}

// transform_embedding_request drops a token input at input[0] and wraps a single string in a list
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
