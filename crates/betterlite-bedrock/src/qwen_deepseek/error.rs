use thiserror::Error;

use crate::invoke_response::error::InvokeReplyError;

// LiteLLM re-raises whatever the response handling raised, so each variant stands for one Python exception.
#[derive(Debug, Error)]
pub enum QwenReplyError {
    #[error("the response body is not valid JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the response body is not a JSON object")]
    NotObject,
    #[error("the {field} field is not a string")]
    NotText { field: &'static str },
    #[error("the usage field is not a JSON object")]
    UsageNotObject,
    #[error("the usage field {name} is not an integer")]
    UsageCount { name: &'static str },
}

#[derive(Debug, Error)]
pub enum DeepseekReplyError {
    #[error(transparent)]
    Base(#[from] InvokeReplyError),
    // The base reports every family through its own wire, and only a chat completion carries the message rebuilt below.
    #[error("the DeepSeek reply is not a chat completion")]
    NotChat,
    #[error("the request prompt is not a string")]
    PromptNotText,
    #[error("the reply token count {name} is out of range")]
    UsageRange { name: &'static str },
}
