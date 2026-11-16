use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingResponse};
use indexmap::IndexMap;

use crate::credentials::headers::{HeaderLookup, process_azure_headers};

use super::client::Headers;
use super::error::EmbeddingError;

const DEFAULT_STATUS: u16 = 500;
const ERROR_STATUS: u16 = 422;
const ERROR_MESSAGE: &str = "Error in response object";

// the response object the caller hands in; only an EmbeddingResponse takes the embedding projection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelResponse {
    Embedding,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingReply {
    pub model: String,
    pub object: String,
    pub data: Vec<EmbeddingRecord>,
    pub usage: ReplyUsage,
    pub response_headers: BTreeMap<String, String>,
    // only the async path stores the raw headers in the hidden params
    pub hidden_headers: Option<Headers>,
}

// convert_to_model_response_object with response_type="embedding" and the Azure processed headers
#[must_use]
pub fn convert_embedding(
    response: EmbeddingResponse,
    headers: Headers,
    keep_hidden_headers: bool,
) -> EmbeddingReply {
    let pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    EmbeddingReply {
        model: response.model,
        object: response.object,
        data: response.data,
        usage: ReplyUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
        },
        response_headers: process_azure_headers(&pairs, HeaderLookup::Exact),
        hidden_headers: keep_hidden_headers.then_some(headers),
    }
}

fn error_status(error: &IndexMap<String, Json>) -> u16 {
    match error.get("code") {
        None => ERROR_STATUS,
        Some(Json::Number(code)) => code
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(DEFAULT_STATUS),
        Some(_other) => DEFAULT_STATUS,
    }
}

// the "error" check that opens convert_to_model_response_object: only a meaningful error raises
#[must_use]
pub fn response_error(body: &[u8]) -> Option<EmbeddingError> {
    let Ok(Json::Object(fields)) = serde_json::from_slice::<Json>(body) else {
        return None;
    };
    match fields.get("error")? {
        Json::Null => None,
        Json::Text(text) if text.is_empty() => None,
        Json::Object(error) => {
            let message = error.get("message");
            let said_something = match message {
                None | Some(Json::Null) => false,
                Some(Json::Text(text)) => !text.is_empty(),
                Some(_other) => true,
            };
            let code_given = error.get("code").is_some_and(|code| *code != Json::Null);
            let text = match message {
                None => ERROR_MESSAGE.to_owned(),
                Some(Json::Text(text)) => text.clone(),
                Some(other) => serde_json::to_string(other).unwrap_or_default(),
            };
            (said_something || code_given).then(|| EmbeddingError::Response {
                status_code: error_status(error),
                message: text,
            })
        }
        Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Some(EmbeddingError::Response {
                status_code: ERROR_STATUS,
                message: ERROR_MESSAGE.to_owned(),
            })
        }
    }
}
