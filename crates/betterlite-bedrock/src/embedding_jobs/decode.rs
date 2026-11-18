use betterlite_models::decisions::Json;

use super::error::EmbeddingJobsError;

// LiteLLM calls response.json() and hands any parsed JSON on, so a nonobject body is a success
pub fn decode_async_embedding_status(bytes: &[u8]) -> Result<Json, EmbeddingJobsError> {
    match serde_json::from_slice(bytes) {
        Ok(status) => Ok(status),
        Err(source) => Err(EmbeddingJobsError::Decode { source }),
    }
}
