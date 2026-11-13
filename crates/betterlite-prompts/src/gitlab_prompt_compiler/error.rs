use thiserror::Error;

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
    Manager(#[from] ManagerError),
}

#[derive(Debug, Error)]
pub enum GetPromptError {
    #[error(transparent)]
    Manager(#[from] ManagerError),
    #[error("prompt values are invalid: {0}")]
    Json(#[from] serde_json::Error),
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
