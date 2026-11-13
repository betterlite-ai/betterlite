use thiserror::Error;

use crate::batches::records::BatchObject;

pub const MAX_BATCH_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum BatchDecodeError {
    #[error("the batch answer is {size} bytes, over the {MAX_BATCH_BYTES} byte limit")]
    TooLarge { size: usize },
    #[error("the answer is not a batch object: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLMBatch.model_validate over the SDK's model_dump: unknown keys stay, a wrong type or status is an error
pub fn decode_batch_object(body: &[u8]) -> Result<BatchObject, BatchDecodeError> {
    if body.len() > MAX_BATCH_BYTES {
        return Err(BatchDecodeError::TooLarge { size: body.len() });
    }
    serde_json::from_slice(body).map_err(|source| BatchDecodeError::Json { source })
}
