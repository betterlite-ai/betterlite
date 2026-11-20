use crate::prepared::PreparedRequest;

use super::call::{EmbeddingCall, ResourceCall};
use super::error::DispatchError;

const PROVIDER: &str = "perplexity";

mod tests;

// main.py's perplexity embedding arm (L7335-L7348); B5834 builds the shared HTTP embedding handler here
#[rustfmt::skip]
pub const fn embedding(_call: &EmbeddingCall) -> Result<EmbeddingCall, DispatchError> {
    Err(DispatchError::NotBuilt {
        provider: PROVIDER,
        operation: "embedding",
    })
}

// the Perplexity Responses config B5360's tag selects; B5834 builds it here
#[rustfmt::skip]
pub const fn responses(_call: &ResourceCall<'_>) -> Result<PreparedRequest, DispatchError> {
    Err(DispatchError::NotBuilt {
        provider: PROVIDER,
        operation: "responses",
    })
}

// the Perplexity search config B5360's tag selects; B5834 builds it here
#[rustfmt::skip]
pub const fn search(_call: &ResourceCall<'_>) -> Result<PreparedRequest, DispatchError> {
    Err(DispatchError::NotBuilt {
        provider: PROVIDER,
        operation: "search",
    })
}

// the Perplexity decisions config B5360's tag selects; B5834 builds it here
#[rustfmt::skip]
pub const fn decisions(_call: &ResourceCall<'_>) -> Result<PreparedRequest, DispatchError> {
    Err(DispatchError::NotBuilt {
        provider: PROVIDER,
        operation: "decisions",
    })
}
