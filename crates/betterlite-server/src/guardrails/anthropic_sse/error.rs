use betterlite_core::api::messages_chat_output::error::ChatOutputError;
use betterlite_core::api::messages_fake_stream::MessagesFakeStreamError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnthropicSseError {
    #[error(transparent)]
    Output(#[from] ChatOutputError),
    #[error(transparent)]
    FakeStream(#[from] MessagesFakeStreamError),
    #[error("an Anthropic frame could not be written as JSON")]
    Json(#[from] serde_json::Error),
}
