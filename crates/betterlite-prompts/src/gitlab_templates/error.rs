use thiserror::Error;

use super::parse::ParseError;
use super::sandbox::SandboxError;
use crate::gitlab_client::error::GitLabClientError;

#[derive(Debug, Error)]
pub enum LoadFailure {
    #[error(transparent)]
    Fetch(#[from] GitLabClientError),
    #[error(transparent)]
    Parse(#[from] ParseError),
}

#[derive(Debug, Error)]
pub enum ManagerError {
    #[error(transparent)]
    Client(#[from] GitLabClientError),
    #[error("Failed to load prompt '{prompt_id}' from GitLab: {source}")]
    Load {
        prompt_id: String,
        #[source]
        source: LoadFailure,
    },
    #[error("Template '{0}' not found")]
    NotFound(String),
    #[error(transparent)]
    Render(#[from] SandboxError),
}
