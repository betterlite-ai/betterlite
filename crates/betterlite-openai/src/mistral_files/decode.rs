use crate::api::batch_records::{BatchRecordsError, WireDecodeLimits};
use serde::de::DeserializeOwned;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum MistralDecodeError {
    #[error(transparent)]
    Limit(#[from] BatchRecordsError),
    #[error("the Mistral resource reply is not the expected JSON: {0}")]
    Decode(#[from] serde_json::Error),
}
pub fn decode_mistral_resource<R: DeserializeOwned>(
    bytes: &[u8],
    limits: &WireDecodeLimits,
) -> Result<R, MistralDecodeError> {
    if bytes.len() > limits.max_json_bytes {
        return Err(BatchRecordsError::JsonTooLarge {
            limit: limits.max_json_bytes,
        }
        .into());
    }
    Ok(serde_json::from_slice(bytes)?)
}
