use thiserror::Error;

use crate::api::vector_store_registry::error::VectorStoreRegistryError;

#[derive(Debug, Error)]
pub enum VectorToolSelectionError {
    #[error(transparent)]
    Registry(#[from] VectorStoreRegistryError),
    #[error("the stored litellm_params value is not an object")]
    StoredParams,
    #[error("the value could not be encoded as JSON")]
    Json(#[from] serde_json::Error),
}
