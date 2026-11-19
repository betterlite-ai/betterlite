use thiserror::Error;

use crate::api::vector_store_http_write::error::WriteError;

// validate_create_vector_store's refusal: the handler lets it out before it builds a header or a URL
#[derive(Debug, Error)]
#[error("{message}")]
pub struct CapabilityError {
    pub message: String,
}

#[derive(Debug, Error)]
pub enum CreateReadError {
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
}
