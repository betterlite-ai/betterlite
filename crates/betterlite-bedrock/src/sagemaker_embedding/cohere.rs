use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::decode::populate_embedding_response;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
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
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Result<Object, CohereRequestError> {
    let texts = match input {
        EmbeddingInput::Text(text) => vec![text.clone()],
        EmbeddingInput::Texts(texts) => texts.clone(),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
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
    logged_input: Option<&EmbeddingInput>,
    request_data: &Object,
    encoding: &dyn TextEncoding,
) -> Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError> {
    let decoded = serde_json::from_slice(body)?;
    let input = reply_input(logged_input, request_data)?;
    populate_embedding_response(&decoded, model, &input, encoding)
}

fn reply_input(
    logged_input: Option<&EmbeddingInput>,
    request_data: &Object,
) -> Result<Vec<String>, CohereLegacyEmbeddingError> {
    let mut chosen = logged_input.filter(|input| has_value(input)).cloned();
    if chosen.is_none() {
        for key in ["texts", "images"] {
            let Some(value) = request_data.get(key).filter(|value| value.is_truthy()) else {
                continue;
            };
            let value = serde_json::to_value(value).map_err(|_error| {
                CohereLegacyEmbeddingError::Usage(CohereEmbeddingError::InputNotText)
            })?;
            chosen = Some(
                serde_json::from_value::<EmbeddingInput>(value).map_err(|_source| {
                    CohereLegacyEmbeddingError::Usage(CohereEmbeddingError::InputNotText)
                })?,
            );
            break;
        }
    }
    match chosen {
        None => Ok(Vec::new()),
        Some(EmbeddingInput::Text(text)) => Ok(vec![text]),
        Some(EmbeddingInput::Texts(texts)) => Ok(texts),
        Some(EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_)) => Err(
            CohereLegacyEmbeddingError::Usage(CohereEmbeddingError::InputNotText),
        ),
    }
}

// Cohere's input choice skips an empty value, as the pinned code's `or` chain does
fn has_value(input: &EmbeddingInput) -> bool {
    match input {
        EmbeddingInput::Text(text) => !text.is_empty(),
        EmbeddingInput::Texts(texts) => !texts.is_empty(),
        EmbeddingInput::Tokens(tokens) => !tokens.is_empty(),
        EmbeddingInput::TokenLists(lists) => !lists.is_empty(),
    }
}
mod tests;
