use std::convert::Infallible;

use super::super::error::VertexDiscoverySearchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStep {
    Request,
    Response,
}

// LiteLLM's create request and response transforms both raise a bare NotImplementedError
pub fn transform_create_store(step: CreateStep) -> Result<Infallible, VertexDiscoverySearchError> {
    tracing::debug!(
        ?step,
        "Vertex AI Search vector store creation is not supported"
    );
    Err(VertexDiscoverySearchError::CreateUnsupported)
}
