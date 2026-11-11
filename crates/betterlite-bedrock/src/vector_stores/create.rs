use std::convert::Infallible;

use crate::vector_stores::error::VectorStoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStep {
    Request,
    Response,
}

// LiteLLM's create request and response transforms both raise a bare NotImplementedError
pub fn transform_create_vector_store(step: CreateStep) -> Result<Infallible, VectorStoreError> {
    tracing::debug!(?step, "Bedrock knowledge base creation is not supported");
    Err(VectorStoreError::CreateNotSupported)
}
