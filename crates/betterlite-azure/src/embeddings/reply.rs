use std::collections::BTreeMap;

use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingResponse};

use crate::credentials::headers::{HeaderLookup, process_azure_headers};

use super::client::Headers;

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
