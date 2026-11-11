use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

use super::super::error::OllamaError;
use super::super::function::WireMessage;

const REQUIRED_CHUNK_FIELDS: [&str; 3] = ["message", "done", "model"];

#[derive(Debug, Deserialize)]
pub struct RawChunk {
    pub(super) model: String,
    pub(super) message: WireMessage,
    pub(super) done: bool,
    #[serde(default)]
    pub(super) done_reason: Option<String>,
    #[serde(default)]
    pub(super) prompt_eval_count: Option<u64>,
    #[serde(default)]
    pub(super) eval_count: Option<u64>,
}

// LiteLLM's chunk_parser: a message, done and model key raise the 400 OllamaError, anything else the generic 500
pub fn decode_chunk(bytes: &[u8]) -> Result<RawChunk, OllamaError> {
    let parsed =
        serde_json::from_slice::<Json>(bytes).map_err(|source| OllamaError::Shape { source })?;
    let Json::Object(object) = parsed else {
        return Err(OllamaError::ChunkField { field: "chunk" });
    };
    for field in REQUIRED_CHUNK_FIELDS {
        if !object.contains_key(field) {
            return Err(OllamaError::ChunkMissing { field });
        }
    }
    let wire = serde_json::to_string(&object).map_err(|source| OllamaError::Shape { source })?;
    serde_json::from_str(&wire).map_err(|source| OllamaError::Shape { source })
}
