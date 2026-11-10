use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::huggingface_chat::error::get_error_class;
use crate::huggingface_embedding::error::HuggingfaceEmbeddingError;
use crate::openai_base_config::Headers;

pub const MAX_RESPONSE_BYTES: usize = 0x400_0000;

// Items comes first so a one-element list is never read as the single-field structs below
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingReply {
    Items(Vec<Value>),
    Failure(FailureReply),
    Similarities(SimilaritiesReply),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FailureReply {
    pub error: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SimilaritiesReply {
    pub similarities: Vec<(usize, Value)>,
}

pub trait OrdinaryEncoder {
    fn encode_ordinary(&self, text: &str) -> usize;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingRecord>,
    pub model: String,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingRecord {
    pub object: String,
    pub index: usize,
    pub embedding: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

pub fn decode_embedding_response(
    bytes: &[u8],
) -> Result<EmbeddingReply, HuggingfaceEmbeddingError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(HuggingfaceEmbeddingError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    serde_json::from_slice(bytes)
        .map_err(|source| HuggingfaceEmbeddingError::ResponseDecode { source })
}

pub fn process_embedding_response(
    reply: &EmbeddingReply,
    model: &str,
    input: &[String],
    encoder: &dyn OrdinaryEncoder,
) -> Result<EmbeddingResponse, HuggingfaceEmbeddingError> {
    let data = match reply {
        EmbeddingReply::Failure(failure) => return Err(failure_error(&failure.error)),
        EmbeddingReply::Similarities(similarities) => similarities
            .similarities
            .iter()
            .map(|(index, embedding)| EmbeddingRecord {
                object: "embedding".to_owned(),
                index: *index,
                embedding: embedding.clone(),
            })
            .collect(),
        EmbeddingReply::Items(items) => {
            let mut records = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                records.push(EmbeddingRecord {
                    object: "embedding".to_owned(),
                    index,
                    embedding: flattened(item, index)?,
                });
            }
            records
        }
    };
    let tokens = token_count(input, encoder)?;
    Ok(EmbeddingResponse {
        object: "list".to_owned(),
        data,
        model: model.to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens: tokens,
            completion_tokens: tokens,
            total_tokens: tokens,
        },
    })
}

fn failure_error(error: &Value) -> HuggingfaceEmbeddingError {
    let message = match error {
        Value::String(message) => message.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            error.to_string()
        }
    };
    HuggingfaceEmbeddingError::Huggingface(get_error_class(&message, 500, &Headers::new()))
}

fn token_count(
    input: &[String],
    encoder: &dyn OrdinaryEncoder,
) -> Result<usize, HuggingfaceEmbeddingError> {
    let mut total = 0_usize;
    for text in input {
        total = total
            .checked_add(encoder.encode_ordinary(text))
            .ok_or(HuggingfaceEmbeddingError::UsageOverflow)?;
    }
    Ok(total)
}

fn flattened(item: &Value, index: usize) -> Result<Value, HuggingfaceEmbeddingError> {
    match item {
        Value::Number(_) => Ok(item.clone()),
        Value::Array(items) => match items.first() {
            Some(Value::Number(_)) => Ok(item.clone()),
            Some(Value::Array(inner)) => inner
                .first()
                .cloned()
                .ok_or(HuggingfaceEmbeddingError::EmbeddingShape { index }),
            Some(_) | None => Err(HuggingfaceEmbeddingError::EmbeddingShape { index }),
        },
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Object(_) => {
            Err(HuggingfaceEmbeddingError::EmbeddingShape { index })
        }
    }
}
