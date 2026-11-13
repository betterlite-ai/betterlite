use thiserror::Error;

use crate::openai_base_config::forwarding::{ErrorRequest, ErrorResponse};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct OpenAiLikeError {
    pub status_code: u16,
    pub message: String,
}

impl OpenAiLikeError {
    // LiteLLM attaches a request that never went anywhere, so the error can be turned into a response
    #[must_use]
    pub fn response(&self) -> ErrorResponse {
        ErrorResponse {
            status_code: self.status_code,
            request: ErrorRequest {
                method: String::from("POST"),
                url: String::from("https://www.litellm.ai"),
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum CompatibleWireError {
    #[error("the reply is not JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("the reply is not a chat completion: {0}")]
    Reply(#[source] serde_json::Error),
}
