use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use serde::Serialize;

use betterlite_prompts::arguments::json::{Json, Object};

use super::error::TitanEmbeddingsError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TitanEmbeddingResponse {
    pub object: &'static str,
    pub data: Vec<EmbeddingRecord>,
    pub model: String,
    pub usage: TitanUsage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TitanUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub prompt_tokens_details: Option<TitanPromptTokensDetails>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TitanPromptTokensDetails {
    pub image_count: u32,
}

// the envelope G1, V2 and multimodal share; the caller names how one reply holds its vector
pub fn transform_response(
    response_list: &[Object],
    model: &str,
    embedding: fn(&Object) -> Result<EmbeddingVector, TitanEmbeddingsError>,
) -> Result<TitanEmbeddingResponse, TitanEmbeddingsError> {
    let mut data = Vec::with_capacity(response_list.len());
    let mut prompt_tokens = 0_u64;
    for (index, reply) in response_list.iter().enumerate() {
        let vector = embedding(reply)?;
        let count = token_count(reply)?;
        prompt_tokens = prompt_tokens
            .checked_add(count)
            .ok_or(TitanEmbeddingsError::TokenOverflow)?;
        let Ok(position) = u32::try_from(index) else {
            return Err(TitanEmbeddingsError::IndexOverflow);
        };
        data.push(EmbeddingRecord {
            object: String::from("embedding"),
            embedding: vector,
            index: position,
        });
    }
    Ok(TitanEmbeddingResponse {
        object: "list",
        data,
        model: model.to_owned(),
        usage: TitanUsage {
            prompt_tokens,
            completion_tokens: 0,
            total_tokens: prompt_tokens,
            prompt_tokens_details: None,
        },
    })
}

// AmazonTitanG1EmbeddingResponse and AmazonTitanMultimodalEmbeddingResponse read one required embedding column
pub fn plain_embedding(reply: &Object) -> Result<EmbeddingVector, TitanEmbeddingsError> {
    let Some(embedding) = reply.get("embedding") else {
        return Err(TitanEmbeddingsError::MissingField("embedding"));
    };
    embedding_vector(embedding, "embedding")
}

// litellm.types.utils.Embedding declares embedding as list | str, so an encoded vector rides through as a string
pub fn embedding_vector(
    value: &Json,
    field: &'static str,
) -> Result<EmbeddingVector, TitanEmbeddingsError> {
    match value {
        Json::Text(encoded) => Ok(EmbeddingVector::Base64(encoded.clone())),
        Json::List(items) => {
            let mut numbers = Vec::with_capacity(items.len());
            for item in items {
                let Json::Number(number) = item else {
                    return Err(TitanEmbeddingsError::MalformedEmbedding(field));
                };
                numbers.push(number.clone());
            }
            Ok(EmbeddingVector::Floats(numbers))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            Err(TitanEmbeddingsError::MalformedEmbedding(field))
        }
    }
}

fn token_count(reply: &Object) -> Result<u64, TitanEmbeddingsError> {
    let Some(value) = reply.get("inputTextTokenCount") else {
        return Err(TitanEmbeddingsError::MissingField("inputTextTokenCount"));
    };
    let Json::Number(number) = value else {
        return Err(TitanEmbeddingsError::MalformedTokenCount);
    };
    number
        .as_u64()
        .ok_or(TitanEmbeddingsError::MalformedTokenCount)
}
