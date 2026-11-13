use thiserror::Error;

use crate::bitbucket_prompt_manager::error::ManagerError;

#[derive(Debug, Error)]
pub enum BitBucketFailure {
    #[error("Prompt template '{prompt_id}' not found")]
    TemplateNotFound { prompt_id: String },
    #[error(transparent)]
    Manager(#[from] ManagerError),
}

#[derive(Debug, Error)]
pub enum BitBucketPromptError {
    #[error("prompt_id is required for BitBucket prompt manager")]
    PromptIdRequired,
    #[error("Error compiling prompt '{prompt_id}': {source}")]
    Compile {
        prompt_id: String,
        #[source]
        source: BitBucketFailure,
    },
}
