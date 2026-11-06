use thiserror::Error;

use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_templates::error::ManagerError;

#[derive(Debug, Error)]
pub enum InitializerError {
    #[error("gitlab_config is required for gitlab prompt integration")]
    MissingConfig,
    #[error("litellm_params.gitlab_config is invalid: {0}")]
    Config(#[from] serde_json::Error),
    #[error("litellm_params.git_ref must be a string")]
    RefNotString,
    #[error(transparent)]
    Client(#[from] GitLabClientError),
}

#[derive(Debug, Error)]
pub enum GetPromptError {
    #[error(transparent)]
    Manager(#[from] ManagerError),
    #[error("prompt variables are invalid: {0}")]
    Variables(#[from] serde_json::Error),
    #[error("the GitLab prompt manager is unavailable")]
    Unavailable,
    #[error("Prompt template '{prompt_id}' not found")]
    TemplateNotFound { prompt_id: String },
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("prompt_id is required for GitLab prompt manager")]
    PromptIdRequired,
    #[error("Error compiling prompt '{prompt_id}': {source}")]
    Compile {
        prompt_id: String,
        #[source]
        source: GetPromptError,
    },
}
