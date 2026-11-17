use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::decode::populate_embedding_response;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::titan_embeddings::cohere as bedrock_cohere;

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["encoding_format", "dimensions", "input_type"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CohereRequestError {
    #[error("Input must be a list of strings")]
    InputNotText,
}

// map_openai_params: the Bedrock mapper runs first, then input_type is copied over unchanged
#[must_use]
pub fn map_openai_params(non_default_params: &Object, optional_params: Object) -> Object {
    let mut mapped = bedrock_cohere::map_openai_params(non_default_params, optional_params);
    if let Some(input_type) = non_default_params.get("input_type") {
        mapped.insert(String::from("input_type"), input_type.clone());
    }
    mapped
}

// transform_embedding_request: only a text or a list of texts reaches the Cohere body
pub fn transform_request(
    model: &str,
    input: &Json,
    optional_params: &Object,
) -> Result<Object, CohereRequestError> {
    let texts = match input {
        Json::Text(text) => vec![text.clone()],
        Json::List(items) => {
            if matches!(
                items.first(),
                Some(
                    Json::List(_) | Json::Number(_) | Json::Bool(_) | Json::Null | Json::Object(_)
                )
            ) {
                return Err(CohereRequestError::InputNotText);
            }
            let mut texts = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(text) = item else {
                    return Err(CohereRequestError::InputNotText);
                };
                texts.push(text.clone());
            }
            texts
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            return Err(CohereRequestError::InputNotText);
        }
    };
    Ok(bedrock_cohere::transform_request(
        model,
        &texts,
        optional_params,
    ))
}

// transform_embedding_response: the logged input is preferred over the body's texts and images
pub fn transform_response(
    body: &[u8],
    model: &str,
    logged_input: Option<&Json>,
    request_data: &Object,
    encoding: &dyn TextEncoding,
) -> Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError> {
    let decoded: Json = serde_json::from_slice(body)?;
    let input = reply_input(logged_input, request_data)?;
    populate_embedding_response(&decoded, model, &input, encoding)
}

fn reply_input(
    logged_input: Option<&Json>,
    request_data: &Object,
) -> Result<Vec<String>, CohereLegacyEmbeddingError> {
    let chosen = [
        logged_input,
        request_data.get("texts"),
        request_data.get("images"),
    ]
    .into_iter()
    .flatten()
    .find(|value| value.is_truthy());
    let texts = match chosen {
        None => Vec::new(),
        Some(Json::Text(text)) => vec![text.clone()],
        Some(Json::List(items)) => {
            let mut texts = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(text) = item else {
                    return Err(CohereLegacyEmbeddingError::Usage(
                        CohereEmbeddingError::InputNotText,
                    ));
                };
                texts.push(text.clone());
            }
            texts
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            return Err(CohereLegacyEmbeddingError::Usage(
                CohereEmbeddingError::InputNotText,
            ));
        }
    };
    Ok(texts)
}

mod tests;
