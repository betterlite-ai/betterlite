use betterlite_logging::azure_storage_logging::signing::SigningError;
use betterlite_logging::azure_storage_logging::token::TokenError;
use betterlite_logging::azure_storage_logging::transport::StorageTransportError;

#[derive(Debug, thiserror::Error)]
pub enum AzureFileStorageError {
    #[error("the Azure storage request could not be signed")]
    Signing(#[from] SigningError),
    #[error("the Azure storage request could not be sent")]
    Transport(#[from] StorageTransportError),
    #[error("the Azure storage token could not be obtained")]
    Token(#[from] TokenError),
    #[error("the Azure storage service answered with status {status} for {operation}")]
    Status {
        operation: &'static str,
        status: u16,
    },
    #[error("Invalid Azure Blob Storage URL: {url}")]
    InvalidUrl { url: String },
    #[error("Invalid Azure Blob Storage URL format: {url}")]
    InvalidUrlFormat { url: String },
    #[error("Filesystem {file_system} does not exist")]
    MissingFileSystem { file_system: String },
    #[error("the Azure storage path {path} has an empty directory name")]
    EmptyDirectory { path: String },
}
