use betterlite_openai::api::responses_shapes::wire::ShapeDecodeError;
use thiserror::Error;

use super::access::PollDenial;
use super::cache::PollingCacheError;

#[derive(Debug, Error)]
pub enum PollingError {
    #[error(transparent)]
    Cache(#[from] PollingCacheError),
    #[error(transparent)]
    Decode(#[from] ShapeDecodeError),
    #[error(transparent)]
    Forbidden(#[from] PollDenial),
    #[error("the polling state could not be serialized: {0}")]
    Encode(#[from] serde_json::Error),
}

// the source takes the patch as keyword arguments, so a field outside its signature cannot occur
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the polling patch has no field named {field}")]
pub struct UnknownPatchField {
    pub field: String,
}
