use crate::prepared::PreparedRequest;

use super::call::{EmbeddingCall, ResourceCall};
use super::error::DispatchError;

const PROVIDER: &str = "perplexity";

mod tests;

const fn not_built(operation: &'static str) -> DispatchError {
    DispatchError::NotBuilt {
        provider: PROVIDER,
        operation,
    }
}

// main.py's perplexity embedding arm (L7335-L7348); B5834 builds the shared HTTP embedding handler here
pub fn embedding(_call: &EmbeddingCall) -> Result<EmbeddingCall, DispatchError> {
    Err(not_built("embedding"))
}

// the Perplexity Responses config B5360's tag selects; B5834 builds it here
pub fn responses(_call: &ResourceCall<'_>) -> Result<PreparedRequest, DispatchError> {
    Err(not_built("responses"))
}

// the Perplexity search config B5360's tag selects; B5834 builds it here
pub fn search(_call: &ResourceCall<'_>) -> Result<PreparedRequest, DispatchError> {
    Err(not_built("search"))
}

// the Perplexity decisions config B5360's tag selects; B5834 builds it here
pub fn decisions(_call: &ResourceCall<'_>) -> Result<PreparedRequest, DispatchError> {
    Err(not_built("decisions"))
}
