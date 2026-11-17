use std::string::FromUtf8Error;

use betterlite_openai::compatible_wire::error::CompatibleWireError;
use thiserror::Error;

use crate::petals::error::PetalsSetupError;
use crate::vllm::error::VllmError;

use super::handler::HandlerError;

// the Lemonade model list and chat reply failures: none of them carries an HTTP status of its own
#[derive(Debug, Error)]
pub enum LemonadeError {
    #[error(
        "Failed to fetch models from Lemonade. Set Lemonade API Base via `LEMONADE_API_BASE` environment variable. Error: {message}"
    )]
    Request { message: String },
    #[error("Failed to fetch models from Lemonade. Status code: {status}, Response: {body}")]
    Status { status: u16, body: String },
    #[error("the {status} Lemonade models answer is not UTF-8 text: {source}")]
    StatusBody {
        status: u16,
        #[source]
        source: FromUtf8Error,
    },
    #[error("the Lemonade models reply is not valid JSON: {source}")]
    Reply {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Lemonade models reply is not an object with a data list")]
    Shape,
    #[error("a Lemonade model entry holds no string id")]
    Entry,
    #[error("the Lemonade chat reply could not be read: {source}")]
    ChatReply {
        #[source]
        source: CompatibleWireError,
    },
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("{0}")]
    Vllm(Box<VllmError>),
    #[error(transparent)]
    Petals(#[from] PetalsSetupError),
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error(transparent)]
    Lemonade(#[from] LemonadeError),
    #[error("api_base is required for triton. Please pass `api_base`")]
    MissingTritonApiBase,
    #[error("Invalid input for ollama embeddings. input must be a string or a list of strings")]
    OllamaEmbeddingInput,
    #[error("the stream value must be a boolean")]
    StreamFlag,
    #[error("the request body could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl DispatchError {
    // the status LiteLLM's provider exception carries for this failure
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Vllm(error) => error.status(),
            Self::Petals(error) => error.status(),
            Self::Handler(error) => error.status_code(),
            Self::Lemonade(_) | Self::Encode { .. } => None,
            Self::MissingTritonApiBase | Self::OllamaEmbeddingInput | Self::StreamFlag => Some(400),
        }
    }
}
