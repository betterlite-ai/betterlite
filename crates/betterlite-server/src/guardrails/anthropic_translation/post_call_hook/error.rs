use betterlite_core::api::messages_chat_output::error::ChatOutputError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PostCallHookError {
    #[error("the response with choices could not be read as an OpenAI ModelResponse")]
    Response {
        #[source]
        source: serde_json::Error,
    },
    #[error("the translated Anthropic message could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Translation(#[from] ChatOutputError),
}
