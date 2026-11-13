use std::convert::Infallible;

use super::error::AzureSearchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStep {
    Request,
    Response,
}

// LiteLLM's create request and response transforms both raise a bare NotImplementedError
pub fn create_vector_store(step: CreateStep) -> Result<Infallible, AzureSearchError> {
    tracing::debug!(?step, "Azure AI Search vector store creation is not supported");
    Err(AzureSearchError::CreateNotImplemented)
}
