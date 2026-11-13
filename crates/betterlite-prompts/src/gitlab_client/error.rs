use std::io;

use thiserror::Error;

use super::payload::PayloadError;

#[derive(Debug, Error)]
pub enum GitLabClientError {
    #[error("project and access_token are required")]
    MissingCredentials,
    #[error("ref must be a non-empty string")]
    EmptyRef,
    #[error("Authentication failed. Check your GitLab token and auth_method.")]
    Unauthorized,
    #[error(
        "Access denied to file '{file_path}'. Check your GitLab permissions for project '{project}'."
    )]
    AccessDenied { file_path: String, project: String },
    #[error("Failed to fetch file '{file_path}': {source}")]
    Fetch {
        file_path: String,
        #[source]
        source: FetchFailure,
    },
    #[error("Failed to fetch file '{file_path}' via JSON endpoint: {source}")]
    JsonFetch {
        file_path: String,
        #[source]
        source: FetchFailure,
    },
}

#[derive(Debug, Error)]
pub enum FetchFailure {
    #[error("the request failed: {0}")]
    Transport(#[source] io::Error),
    #[error("GitLab answered with status {0}")]
    Status(u16),
    #[error(transparent)]
    Payload(#[from] PayloadError),
}
