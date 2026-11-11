use thiserror::Error;

use crate::signing::error::SigningError;

#[derive(Debug, Error)]
pub enum BatchStatusError {
    #[error("Failed to parse Bedrock batch response: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Bedrock batch response is not a JSON object")]
    ResponseShape,
    #[error("the {field} field has the wrong type")]
    Field { field: &'static str },
    #[error("the {field} count does not fit a 64-bit integer")]
    CountRange { field: &'static str },
    #[error("the number does not fit an epoch timestamp")]
    EpochRange,
    #[error("a non-finite number has no epoch value")]
    NotFinite,
    #[error("Invalid batch_id format. Expected ARN, got: {batch_id}")]
    BatchId { batch_id: String },
    #[error("Invalid ARN format: {batch_id}")]
    ArnFormat { batch_id: String },
    #[error("Invalid region in ARN: {batch_id}")]
    Region { batch_id: String },
    #[error("the AWS parameters cannot be encoded for signing")]
    Parameters {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Bedrock request cannot be signed")]
    Signing {
        #[source]
        source: SigningError,
    },
}
