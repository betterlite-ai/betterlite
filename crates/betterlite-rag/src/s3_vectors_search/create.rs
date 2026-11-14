use std::convert::Infallible;

use super::error::S3VectorsSearchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStep {
    Request,
    Response,
}

// LiteLLM's create request and response transforms both raise a bare NotImplementedError
pub fn transform_create_vector_store(step: CreateStep) -> Result<Infallible, S3VectorsSearchError> {
    tracing::debug!(?step, "S3 Vectors store creation is not supported");
    Err(S3VectorsSearchError::CreateNotSupported)
}
