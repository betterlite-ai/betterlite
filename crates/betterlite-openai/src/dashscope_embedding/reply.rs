use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::embedding::{EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector};
use crate::openai_base_config::Headers;

use super::error::DashscopeError;

// an entry the embedding record types cannot hold is an unclassified provider failure, which LiteLLM answers with 500
const SHAPE_FAILURE_STATUS: u16 = 500;

pub struct DashscopeEmbeddingReply {
    pub response: EmbeddingResponse,
    pub id: Option<String>,
}

pub fn decode_dashscope_embedding_reply(
    bytes: &[u8],
    status_code: u16,
    request_model: &str,
) -> Result<DashscopeEmbeddingReply, DashscopeError> {
    let body: Json = serde_json::from_slice(bytes).map_err(|source| DashscopeError {
        status_code,
        message: format!("Failed to parse DashScope response as JSON: {source}"),
        headers: Headers::new(),
    })?;
    let Json::Object(fields) = &body else {
        return Err(shape_error("the reply"));
    };
    if let Some(error) = fields.get("error") {
        return Err(DashscopeError {
            status_code,
            message: error_message(error),
            headers: Headers::new(),
        });
    }
    let data = data_records(fields.get("data"))?;
    let model = match fields.get("model") {
        None | Some(Json::Null) => request_model.to_owned(),
        Some(Json::Text(name)) => name.clone(),
        Some(_) => return Err(shape_error("model")),
    };
    let usage = usage(fields.get("usage"))?;
    let id = match fields.get("id") {
        None | Some(Json::Null) => None,
        Some(Json::Text(value)) => Some(value.clone()),
        Some(_) => return Err(shape_error("id")),
    };
    Ok(DashscopeEmbeddingReply {
        response: EmbeddingResponse {
            object: "list".to_owned(),
            data,
            model,
            usage,
        },
        id,
    })
}

fn error_message(error: &Json) -> String {
    match error {
        Json::Object(fields) => match fields.get("message") {
            Some(Json::Text(message)) => message.clone(),
            _ => serde_json::to_string(error).unwrap_or_default(),
        },
        _ => serde_json::to_string(error).unwrap_or_default(),
    }
}

fn data_records(data: Option<&Json>) -> Result<Vec<EmbeddingRecord>, DashscopeError> {
    match data {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => items.iter().map(embedding_record).collect(),
        Some(_) => Err(shape_error("data")),
    }
}

fn embedding_record(item: &Json) -> Result<EmbeddingRecord, DashscopeError> {
    let Json::Object(fields) = item else {
        return Err(shape_error("data"));
    };
    let object = match fields.get("object") {
        Some(Json::Text(text)) => text.clone(),
        _ => return Err(shape_error("data.object")),
    };
    let index = match fields.get("index") {
        Some(Json::Number(number)) => number.as_u64().and_then(|value| u32::try_from(value).ok()),
        _ => None,
    };
    let Some(index) = index else {
        return Err(shape_error("data.index"));
    };
    let embedding = match fields.get("embedding") {
        Some(Json::Text(text)) => EmbeddingVector::Base64(text.clone()),
        Some(Json::List(values)) => EmbeddingVector::Floats(vector(values)?),
        _ => return Err(shape_error("data.embedding")),
    };
    Ok(EmbeddingRecord {
        object,
        embedding,
        index,
    })
}

fn vector(values: &[Json]) -> Result<Vec<Number>, DashscopeError> {
    values
        .iter()
        .map(|value| match value {
            Json::Number(number) => Ok(number.clone()),
            _ => Err(shape_error("data.embedding")),
        })
        .collect()
}

fn usage(usage: Option<&Json>) -> Result<EmbeddingUsage, DashscopeError> {
    let fields = match usage {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => return Err(shape_error("usage")),
    };
    let prompt_tokens = token_count(fields, "prompt_tokens")?.unwrap_or(0);
    let total_tokens = token_count(fields, "total_tokens")?.unwrap_or(prompt_tokens);
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        completion_tokens_details: None,
        prompt_tokens_details: None,
        total_tokens,
    })
}

fn token_count(
    fields: Option<&IndexMap<String, Json>>,
    name: &str,
) -> Result<Option<u32>, DashscopeError> {
    let Some(fields) = fields else {
        return Ok(None);
    };
    let Some(value) = fields.get(name) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| shape_error(&format!("usage.{name}"))),
        _ => Err(shape_error(&format!("usage.{name}"))),
    }
}

fn shape_error(field: &str) -> DashscopeError {
    DashscopeError {
        status_code: SHAPE_FAILURE_STATUS,
        message: format!("the DashScope embedding reply field `{field}` has the wrong type"),
        headers: Headers::new(),
    }
}
