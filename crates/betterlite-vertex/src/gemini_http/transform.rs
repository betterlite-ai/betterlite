use std::convert::Infallible;

use thiserror::Error;

// LiteLLM's transform_request only raises NotImplementedError, so no body is built here
#[derive(Debug, Error)]
#[error("Vertex AI has a custom implementation of transform_request. Needs sync + async.")]
pub struct TransformRequestUnsupported;

pub fn transform_request() -> Result<Infallible, TransformRequestUnsupported> {
    tracing::debug!("the Vertex Gemini transform_request entry stays unsupported");
    Err(TransformRequestUnsupported)
}
