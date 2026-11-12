use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use serde::Serialize;

use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::input::{EmbeddingInput, ResolvedFiles};
use crate::gemini_embedding_records::usage::{
    EmbeddingUsage, TextTokenCounter, batch_usage, embed_content_usage,
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmbeddingReply {
    pub model: String,
    pub data: Vec<EmbeddingRecord>,
    pub usage: EmbeddingUsage,
}
pub fn process_batch_reply(
    input: &EmbeddingInput,
    model: &str,
    response: &Json,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingReply, GeminiEmbeddingError> {
    let Json::Object(fields) = response else {
        return Err(GeminiEmbeddingError::Reply {
            reason: "the embeddings reply must be an object",
        });
    };
    let Some(Json::List(embeddings)) = fields.get("embeddings") else {
        return Err(GeminiEmbeddingError::Reply {
            reason: "the embeddings reply must hold an embeddings list",
        });
    };
    let mut data = Vec::with_capacity(embeddings.len());
    for (position, embedding) in embeddings.iter().enumerate() {
        data.push(embedding_record(embedding, position)?);
    }
    Ok(EmbeddingReply {
        model: model.to_owned(),
        data,
        usage: batch_usage(input, model, counter)?,
    })
}
pub fn process_embed_content_reply(
    input: &EmbeddingInput,
    model: &str,
    response: &Json,
    resolved_files: &ResolvedFiles,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingReply, GeminiEmbeddingError> {
    let fields = match response {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(GeminiEmbeddingError::Reply {
                reason: "the embeddings reply must be an object",
            });
        }
    };
    let Some(embedding) = fields.get("embedding") else {
        return Err(GeminiEmbeddingError::Reply {
            reason: "the embedContent reply has no embedding",
        });
    };
    Ok(EmbeddingReply {
        model: model.to_owned(),
        data: vec![embedding_record(embedding, 0)?],
        usage: embed_content_usage(
            input,
            model,
            fields.get("usageMetadata"),
            resolved_files,
            counter,
        )?,
    })
}
fn embedding_record(
    embedding: &Json,
    position: usize,
) -> Result<EmbeddingRecord, GeminiEmbeddingError> {
    let Json::Object(fields) = embedding else {
        return Err(GeminiEmbeddingError::Reply {
            reason: "an embeddings entry must be an object",
        });
    };
    let Some(values) = fields.get("values") else {
        return Err(GeminiEmbeddingError::Reply {
            reason: "an embeddings entry has no values",
        });
    };
    let vector = match values {
        Json::Text(encoded) => EmbeddingVector::Base64(encoded.clone()),
        Json::List(items) => {
            let mut numbers = Vec::with_capacity(items.len());
            for item in items {
                let Json::Number(number) = item else {
                    return Err(GeminiEmbeddingError::Reply {
                        reason: "the values list must hold numbers",
                    });
                };
                numbers.push(number.clone());
            }
            EmbeddingVector::Floats(numbers)
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            return Err(GeminiEmbeddingError::Reply {
                reason: "the values value must be a list of numbers",
            });
        }
    };
    let index = u32::try_from(position).map_err(|_error| GeminiEmbeddingError::IndexOverflow)?;
    Ok(EmbeddingRecord {
        object: String::from("embedding"),
        embedding: vector,
        index,
    })
}
