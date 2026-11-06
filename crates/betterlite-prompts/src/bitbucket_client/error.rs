use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use std::string::FromUtf8Error;

use thiserror::Error;

use super::path::PathError;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("workspace, repository, and access_token are required")]
    MissingRequired,
    #[error("the access token cannot be carried in an Authorization header")]
    InvalidAuthorization(#[source] InvalidHeaderValue),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("the file URL is not valid")]
    InvalidUrl(#[source] url::ParseError),
    #[error(
        "Access denied to file '{file_path}'. Check your BitBucket permissions for workspace '{workspace}' and repository '{repository}'."
    )]
    AccessDenied {
        file_path: String,
        workspace: String,
        repository: String,
    },
    #[error("Authentication failed. Check your BitBucket access token and permissions.")]
    AuthenticationFailed,
    #[error("Failed to fetch file '{file_path}': BitBucket answered with status {status}")]
    Status { file_path: String, status: u16 },
    #[error("The file '{file_path}' is not valid UTF-8")]
    NotUtf8 {
        file_path: String,
        #[source]
        source: FromUtf8Error,
    },
    #[error("Error fetching file '{file_path}'")]
    Transport {
        file_path: String,
        #[source]
        source: TransportError,
    },
}
