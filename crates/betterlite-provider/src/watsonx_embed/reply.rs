use std::str;

use betterlite_openai::embedding::EmbeddingVector;
use serde::Deserialize;

use super::error::WatsonxEmbeddingError;

#[derive(Debug, Deserialize)]
pub struct WatsonxEmbeddingReply {
    #[serde(default)]
    pub results: Vec<WatsonxEmbeddingResult>,
    #[serde(default)]
    pub input_token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct WatsonxEmbeddingResult {
    pub embedding: EmbeddingVector,
}

pub fn decode_watsonx_embedding_reply(
    bytes: &[u8],
) -> Result<WatsonxEmbeddingReply, WatsonxEmbeddingError> {
    serde_json::from_slice(bytes).map_err(|source| WatsonxEmbeddingError::InvalidResponse {
        body: body_text(bytes),
        source,
    })
}

fn body_text(bytes: &[u8]) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return format!("{} bytes that are not UTF-8", bytes.len());
    };
    text.to_owned()
}
