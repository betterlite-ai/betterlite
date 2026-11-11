use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use serde::Serialize;
use serde_json::Number;

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
    let embeddings = list_field(response, "embeddings")?;
    let mut data = Vec::with_capacity(embeddings.len());
    for (position, embedding) in embeddings.iter().enumerate() {
        data.push(embedding_record(embedding, position)?);
    }
    let usage = batch_usage(input, model, counter)?;
    Ok(EmbeddingReply {
        model: model.to_owned(),
        data,
        usage,
    })
}

pub fn process_embed_content_reply(
    input: &EmbeddingInput,
    model: &str,
    response: &Json,
    resolved_files: &ResolvedFiles,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingReply, GeminiEmbeddingError> {
    let Json::Object(fields) = response else {
        return Err(GeminiEmbeddingError::ReplyNotObject);
    };
    let Some(embedding) = fields.get("embedding") else {
        return Err(GeminiEmbeddingError::MissingField { field: "embedding" });
    };
    let record = embedding_record(embedding, 0)?;
    let usage = embed_content_usage(
        input,
        model,
        fields.get("usageMetadata"),
        resolved_files,
        counter,
    )?;
    Ok(EmbeddingReply {
        model: model.to_owned(),
        data: vec![record],
        usage,
    })
}

fn list_field<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<&'value [Json], GeminiEmbeddingError> {
    let Json::Object(fields) = value else {
        return Err(GeminiEmbeddingError::ReplyNotObject);
    };
    let Some(found) = fields.get(field) else {
        return Err(GeminiEmbeddingError::MissingField { field });
    };
    let Json::List(items) = found else {
        return Err(GeminiEmbeddingError::NotList { field });
    };
    Ok(items)
}

fn embedding_record(
    embedding: &Json,
    position: usize,
) -> Result<EmbeddingRecord, GeminiEmbeddingError> {
    let Json::Object(fields) = embedding else {
        return Err(GeminiEmbeddingError::NotObject {
            field: "embeddings",
        });
    };
    let Some(values) = fields.get("values") else {
        return Err(GeminiEmbeddingError::MissingField { field: "values" });
    };
    let vector = match values {
        Json::Text(encoded) => EmbeddingVector::Base64(encoded.clone()),
        Json::List(items) => EmbeddingVector::Floats(numbers(items)?),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            return Err(GeminiEmbeddingError::NotNumbers { field: "values" });
        }
    };
    let index = u32::try_from(position).map_err(|_error| GeminiEmbeddingError::IndexOverflow)?;
    Ok(EmbeddingRecord {
        object: String::from("embedding"),
        embedding: vector,
        index,
    })
}

fn numbers(items: &[Json]) -> Result<Vec<Number>, GeminiEmbeddingError> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Json::Number(number) = item else {
            return Err(GeminiEmbeddingError::NotNumbers { field: "values" });
        };
        values.push(number.clone());
    }
    Ok(values)
}
