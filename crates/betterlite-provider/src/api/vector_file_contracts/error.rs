use betterlite_openai::api::vector_file_contracts::records::FileParams;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: FileParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VectorStoreFilesError {
    #[error("api_base is required")]
    ApiBaseRequired,
    #[error(transparent)]
    Upstream(#[from] BaseLlmException),
}
