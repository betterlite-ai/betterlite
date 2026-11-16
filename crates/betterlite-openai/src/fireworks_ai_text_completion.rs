use thiserror::Error;

use crate::text_completion_policy::prompt::PromptError;

pub mod extra_body;
pub mod mapping;
pub mod request;
pub mod supported;

mod tests;

#[derive(Debug, Error)]
pub enum FireworksCompletionError {
    #[error(transparent)]
    Prompt(#[from] PromptError),
}

impl FireworksCompletionError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Prompt(_) => None,
        }
    }
}
