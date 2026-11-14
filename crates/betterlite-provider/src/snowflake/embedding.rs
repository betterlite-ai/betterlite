use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use thiserror::Error;

use super::SnowflakeException;
use crate::api::file_contracts::config::Headers;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingReply {
    pub response: EmbeddingResponse,
    pub hidden_model: String,
}

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("the provider answer could not be read: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider answer could not be written back as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the embedding reply data must be a list")]
    Data,
    #[error("the embedding at index {index} must be an object")]
    Item { index: usize },
    #[error("the embedding at index {index} must be a list of numbers")]
    Shape { index: usize },
    #[error("the embedding at index {index} is empty")]
    Empty { index: usize },
}

#[must_use]
pub fn transform_embedding_request(input: &Json, model: &str, optional_params: &Object) -> Object {
    let mut body = Object::from_iter([
        ("text".to_owned(), input.clone()),
        ("model".to_owned(), Json::Text(model.to_owned())),
    ]);
    body.extend(optional_params.clone());
    body
}

pub fn transform_embedding_response(
    model: &str,
    raw_response: &str,
) -> Result<EmbeddingReply, EmbeddingError> {
    let mut body: Json =
        serde_json::from_str(raw_response).map_err(|source| EmbeddingError::Decode { source })?;
    let Json::Object(fields) = &mut body else {
        return Err(EmbeddingError::Data);
    };
    unwrap_vectors(fields.get_mut("data"))?;
    // LiteLLM's EmbeddingResponse always writes object as "list"
    fields.insert("object".to_owned(), Json::Text("list".to_owned()));
    let response_model = fields
        .get("model")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_owned();
    fields.insert(
        "model".to_owned(),
        Json::Text(format!("snowflake/{response_model}")),
    );
    let encoded =
        serde_json::to_string(&body).map_err(|source| EmbeddingError::Encode { source })?;
    let response: EmbeddingResponse =
        serde_json::from_str(&encoded).map_err(|source| EmbeddingError::Decode { source })?;
    Ok(EmbeddingReply {
        response,
        hidden_model: model.to_owned(),
    })
}

// Cortex nests every vector one level deep, so LiteLLM keeps element 0 and drops the rest
fn unwrap_vectors(data: Option<&mut Json>) -> Result<(), EmbeddingError> {
    let Some(Json::List(items)) = data else {
        return Err(EmbeddingError::Data);
    };
    for (index, item) in items.iter_mut().enumerate() {
        let Json::Object(fields) = item else {
            return Err(EmbeddingError::Item { index });
        };
        let values = match fields.get("embedding") {
            Some(Json::List(values)) => values,
            None | Some(_) => return Err(EmbeddingError::Shape { index }),
        };
        let first = values.first().ok_or(EmbeddingError::Empty { index })?;
        let Json::List(numbers) = first else {
            return Err(EmbeddingError::Shape { index });
        };
        if !numbers.iter().all(|value| matches!(value, Json::Number(_))) {
            return Err(EmbeddingError::Shape { index });
        }
        let flattened = first.clone();
        fields.insert("embedding".to_owned(), flattened);
        // LiteLLM declares Embedding.object as "embedding", so a missing one is filled in
        fields
            .entry("object".to_owned())
            .or_insert_with(|| Json::Text("embedding".to_owned()));
    }
    Ok(())
}

// LiteLLM's get_error_class hands the caller's status, message and headers to SnowflakeException
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: Headers) -> SnowflakeException {
    SnowflakeException {
        status_code,
        message: message.to_owned(),
        headers,
    }
}

#[cfg(test)]
mod tests;
