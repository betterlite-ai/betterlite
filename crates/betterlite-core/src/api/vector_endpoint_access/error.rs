use betterlite_auth::vector_index_admin::IndexAdminError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VectorEndpointAccessError {
    #[error(
        "User does not have permission to call vector store endpoint {index_name}. Ask your administrator to add the necessary permissions to your API key/Team."
    )]
    StoreDenied { index_name: String },
    #[error(
        "User does not have permission to call vector store file endpoint {vector_store_id}. Ask your administrator to add the necessary permissions to your API key/Team."
    )]
    FileDenied { vector_store_id: String },
    #[error(transparent)]
    Admin(#[from] IndexAdminError),
    #[error("failed to read the caller's vector store grants")]
    Grants(#[source] serde_json::Error),
}

impl VectorEndpointAccessError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::StoreDenied { .. } | Self::FileDenied { .. } | Self::Admin(_) => 403,
            Self::Grants(_) => 500,
        }
    }
}
