use betterlite_env::EnvError;
use betterlite_prompts::error::FactoryError;
use http::HeaderMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TritonError {
    #[error("api_base is required")]
    MissingApiBase,
    #[error("Invalid Triton API base: {api_base}")]
    InvalidApiBase { api_base: String },
    #[error("the Triton reply has the wrong shape: {source}")]
    ReplyShape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Triton reply has no outputs")]
    NoOutputs,
    #[error("the Triton request has no messages")]
    NoMessages,
    #[error("{name} must be an integer")]
    NotAnInteger { name: &'static str },
    #[error("{name} must be a boolean")]
    NotABoolean { name: &'static str },
    #[error("Shape must be of length 2.")]
    ShapeLength,
    #[error("shape value {value} cannot be used")]
    ShapeValue { value: i64 },
    #[error("shape count {count} exceeds the {elements} values sent")]
    ShapeExceedsData { count: usize, elements: usize },
    #[error("the embedding token count does not fit in a 32-bit field")]
    UsageOverflow,
    #[error("the reply has more embeddings than a 32-bit index can hold")]
    TooManyEmbeddings,
    #[error("the reply embedding value {value} is not a number")]
    InvalidEmbedding { value: f64 },
    #[error("the prompt factory returned a prompt this endpoint cannot send")]
    UnexpectedPrompt,
    #[error("the Triton prompt could not be rendered: {source}")]
    Prompt {
        #[source]
        source: FactoryError,
    },
    #[error("the Triton environment is invalid: {source}")]
    Environment {
        #[source]
        source: EnvError,
    },
    // both get_error_class and an unreadable reply body raise this one class in LiteLLM
    #[error("Triton answered with status {status_code}: {message}")]
    Api {
        status_code: u16,
        message: String,
        headers: HeaderMap,
    },
}

impl TritonError {
    // an unhandled transform exception ends as LiteLLM's generic 500, so only this class carries a status
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        let Self::Api { status_code, .. } = self else {
            return None;
        };
        Some(*status_code)
    }
}
