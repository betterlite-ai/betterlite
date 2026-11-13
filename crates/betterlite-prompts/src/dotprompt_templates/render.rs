use thiserror::Error;

use super::validation::ValidationError;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("Prompt '{prompt_id}'{version_suffix} not found. Available prompts: {available}")]
    NotFound {
        prompt_id: String,
        version_suffix: String,
        available: String,
    },
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("Error rendering template '{prompt_id}': {source}")]
    Template {
        prompt_id: String,
        #[source]
        source: minijinja::Error,
    },
    #[error("prompt template exceeds its byte budget")]
    TemplateTooLarge,
    #[error("rendered output exceeds its byte budget")]
    OutputTooLarge,
}

mod tests;
