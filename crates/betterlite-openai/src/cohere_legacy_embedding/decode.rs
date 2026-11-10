use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
use serde::Serialize;

use super::error::CohereLegacyEmbeddingError;
use super::{EmbeddingHooks, PostCall};
use crate::cohere_embedding::TextEncoding;
use crate::cohere_embedding::usage::{CohereEmbeddingUsage, calculate_usage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyEmbeddingResponse {
    pub object: String,
    pub data: Vec<LegacyEmbeddingRecord>,
    pub model: String,
    pub usage: CohereEmbeddingUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyEmbeddingRecord {
    pub object: String,
    pub index: usize,
    pub embedding: Json,
    // only a grouped reply labels its rows
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Clone, Copy)]
pub struct DecodeInput<'call> {
    pub body: &'call [u8],
    pub model: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub request_data: &'call Object,
    pub input: &'call [String],
    pub encoding: &'call dyn TextEncoding,
}

// _transform_response: the reply is parsed before post_call, so a malformed body logs nothing
pub fn decode_cohere_legacy_embedding(
    reply: DecodeInput<'_>,
    hooks: &mut dyn EmbeddingHooks,
) -> Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError> {
    let decoded: Json = serde_json::from_slice(reply.body)
        .map_err(|source| CohereLegacyEmbeddingError::Json { source })?;
    hooks.post_call(PostCall {
        input: reply.input,
        api_key: reply.api_key,
        body: reply.request_data,
        original_response: &decoded,
    });
    populate_embedding_response(&decoded, reply.model, reply.input, reply.encoding)
}

// split out so a caller that already logged the reply fills a response without a second post_call
pub fn populate_embedding_response(
    decoded: &Json,
    model: &str,
    input: &[String],
    encoding: &dyn TextEncoding,
) -> Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError> {
    let fields = decoded
        .as_object()
        .ok_or(CohereLegacyEmbeddingError::ReplyNotAnObject)?;
    let embeddings = fields
        .get("embeddings")
        .ok_or(CohereLegacyEmbeddingError::MissingEmbeddings)?;
    let data = match embeddings {
        Json::Object(groups) => grouped_rows(groups)?,
        Json::List(_)
            if fields.get("response_type").and_then(Json::as_str) == Some("embeddings_by_type") =>
        {
            return Err(CohereLegacyEmbeddingError::EmbeddingsNotGrouped);
        }
        Json::List(rows) => flat_rows(rows),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
            return Err(CohereLegacyEmbeddingError::EmbeddingsShape);
        }
    };
    Ok(LegacyEmbeddingResponse {
        object: String::from("list"),
        data,
        model: model.to_owned(),
        usage: calculate_usage(input, encoding, fields.get("meta"))?,
    })
}

// every embedding type numbers its own rows from zero and adds its name as the row type
fn grouped_rows(groups: &Object) -> Result<Vec<LegacyEmbeddingRecord>, CohereLegacyEmbeddingError> {
    let mut records = Vec::new();
    for (name, group) in groups {
        let Json::List(rows) = group else {
            return Err(CohereLegacyEmbeddingError::EmbeddingsShape);
        };
        for (index, embedding) in rows.iter().enumerate() {
            records.push(LegacyEmbeddingRecord {
                object: String::from("embedding"),
                index,
                embedding: embedding.clone(),
                kind: Some(name.clone()),
            });
        }
    }
    Ok(records)
}

fn flat_rows(rows: &[Json]) -> Vec<LegacyEmbeddingRecord> {
    rows.iter()
        .enumerate()
        .map(|(index, embedding)| LegacyEmbeddingRecord {
            object: String::from("embedding"),
            index,
            embedding: embedding.clone(),
            kind: None,
        })
        .collect()
}
