use betterlite_prompts::arguments::json::Json;

use crate::embeddings::error::FoundryEmbeddingError;
use crate::embeddings::reply::FoundryEmbeddingResponse;

const NUM_TOKENS_HEADER: &str = "llm_provider-num_tokens";
const MODEL_GROUP_HEADER: &str = "llm_provider-azureml-model-group";
const MULTILINGUAL_GROUP: &str = "offer-cohere-embed-multili-paygo";
const MULTILINGUAL_MODEL: &str = "Cohere-embed-v3-multilingual";
const ENGLISH_GROUP: &str = "offer-cohere-embed-english-paygo";
const ENGLISH_MODEL: &str = "Cohere-embed-v3-english";

#[must_use]
pub fn map_azure_model_group(model: &str) -> &str {
    match model {
        MULTILINGUAL_GROUP => MULTILINGUAL_MODEL,
        ENGLISH_GROUP => ENGLISH_MODEL,
        _ => model,
    }
}

// LiteLLM reads the two Cohere headers from the additional headers the last conversion left behind
pub fn transform_response(
    response: &mut FoundryEmbeddingResponse,
) -> Result<(), FoundryEmbeddingError> {
    let headers = response.additional_headers.clone();
    let tokens = headers
        .get(NUM_TOKENS_HEADER)
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty());
    if let Some(tokens) = tokens {
        response.usage.prompt_tokens = token_count(tokens)?;
    }
    let group = headers
        .get(MODEL_GROUP_HEADER)
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty());
    if let Some(group) = group {
        response.model = Some(map_azure_model_group(group).to_owned());
    }
    Ok(())
}

// Python's int() trims the string; anything else that is not a token count is a conversion failure
fn token_count(value: &str) -> Result<u32, FoundryEmbeddingError> {
    let Ok(count) = value.trim().parse::<u32>() else {
        return Err(FoundryEmbeddingError::TokenCount {
            value: value.to_owned(),
        });
    };
    Ok(count)
}
