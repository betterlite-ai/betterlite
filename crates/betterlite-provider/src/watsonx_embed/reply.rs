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
        body: String::from_utf8_lossy(bytes).into_owned(),
        source,
    })
}
