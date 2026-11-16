use std::fmt;

use thiserror::Error;

use crate::registry::mutation::prompt_registry_mutation::error::PromptRegistryError;

mod tests;

#[derive(Error)]
pub enum InitPromptsError {
    #[error("prompt at index {index} is not a valid prompt specification")]
    InvalidPrompt {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("prompt {prompt_id} could not be initialized: {source}")]
    Initialize {
        prompt_id: String,
        #[source]
        source: PromptRegistryError,
    },
}

impl fmt::Debug for InitPromptsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // serde quotes the offending value, which can be a prompt body or a secret
        fmt::Display::fmt(self, f)
    }
}
