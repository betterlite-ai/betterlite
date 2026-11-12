use thiserror::Error;

use crate::openai_base_config::Headers;

// OobaboogaError is a BaseLLMException, so a struct literal is its constructor and its get_error_class
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct OobaboogaError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
}

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error(transparent)]
    Server(#[from] OobaboogaError),
    #[error("the reply does not have the expected shape: {source}")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the reply has no {field}")]
    Missing { field: &'static str },
}
