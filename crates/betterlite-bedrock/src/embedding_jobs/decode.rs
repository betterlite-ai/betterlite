use betterlite_models::decisions::Json;

use super::error::EmbeddingJobsError;

pub fn decode_async_embedding_status(bytes: &[u8]) -> Result<Json, EmbeddingJobsError> {
    serde_json::from_slice(bytes).map_err(|source| EmbeddingJobsError::Decode { source })
}
