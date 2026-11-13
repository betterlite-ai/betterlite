use crate::mistral_files::error::MistralFilesError;
use crate::mistral_identity::error::MistralIdentityError;
use betterlite_http::error::HttpError;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum MistralBatchesError {
    #[error(transparent)]
    Identity(#[from] MistralIdentityError),
    #[error(transparent)]
    Files(#[from] MistralFilesError),
    #[error("input_file_id is required to create a Mistral batch job")]
    InputFileRequired,
    #[error("endpoint is required to create a Mistral batch job")]
    EndpointRequired,
    #[error("the Mistral batch URL cannot carry its batch_id: {source}")]
    Path {
        #[source]
        source: HttpError,
    },
}
impl MistralBatchesError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Files(error) => error.status_code(),
            Self::Identity(MistralIdentityError::Api { status, .. }) => Some(*status),
            Self::Identity(_)
            | Self::Path { .. }
            | Self::InputFileRequired
            | Self::EndpointRequired => None,
        }
    }
}
