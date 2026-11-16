use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_openai::watsonx_audio::error::body_text;
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

// a reply that is valid JSON but not an object must fail, where serde would read an array as the struct's fields
pub fn decode_watsonx_embedding_reply(
    bytes: &[u8],
) -> Result<WatsonxEmbeddingReply, WatsonxEmbeddingError> {
    let invalid = |source: serde_json::Error| WatsonxEmbeddingError::InvalidResponse {
        body: body_text(bytes),
        source,
    };
    serde_json::from_slice::<BTreeMap<String, Json>>(bytes).map_err(invalid)?;
    serde_json::from_slice(bytes).map_err(invalid)
}
