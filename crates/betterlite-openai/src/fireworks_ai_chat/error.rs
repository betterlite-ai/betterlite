use thiserror::Error;

use betterlite_prompts::definitions::error::DefinitionsError;

use crate::chat_content::ChatContentError;
use crate::fireworks_ai_common::error::{FireworksError, MissingApiKey};

#[derive(Debug, Error)]
pub enum FireworksChatError {
    #[error(transparent)]
    MissingApiKey(#[from] MissingApiKey),
    #[error(transparent)]
    Response(#[from] FireworksError),
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error(transparent)]
    Definitions(#[from] DefinitionsError),
    #[error("the reply is not a chat completion: {source}")]
    Reply {
        #[source]
        source: serde_json::Error,
    },
    #[error("a chat message could not be converted for the Fireworks cleanup: {source}")]
    MessageShape {
        #[source]
        source: serde_json::Error,
    },
    #[error("tools[{index}] is not an object with a function")]
    Tool { index: usize },
    #[error("tools must be a list")]
    Tools,
    #[error("stream_options must be an object")]
    StreamOptions,
    #[error(
        "Fireworks AI chat completions does not support file content blocks. For PDFs, convert pages to images and send image_url blocks to a Fireworks vision model, or extract text before calling a text-only model."
    )]
    FileContent,
    #[error(
        "Fireworks AI model {model} does not support image inputs. Use a Fireworks vision model or remove image_url content blocks."
    )]
    ImageContent { model: String },
    #[error(
        "FIREWORKS_API_BASE or FIREWORKS_API_KEY is not set. Please set the environment variable, to query Fireworks AI's `/models` endpoint."
    )]
    ModelListCredentials,
    #[error(
        "FIREWORKS_ACCOUNT_ID is not set. Please set the environment variable, to query Fireworks AI's `/models` endpoint."
    )]
    AccountId,
    #[error("Failed to fetch models from Fireworks AI. Status code: {status}, Response: {body}")]
    ModelListStatus { status: u16, body: String },
    #[error("the models reply is not valid JSON: {source}")]
    ModelsDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the models reply is larger than 1 MiB")]
    ModelsTooLarge,
}

impl FireworksChatError {
    // a ValueError in LiteLLM carries no status, and helper cases compare the status only
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey(_) => None,
            Self::Response(error) => Some(error.status_code),
            Self::Content(error) => Some(error.status_code()),
            Self::Definitions(_)
            | Self::FileContent
            | Self::ImageContent { .. }
            | Self::Tool { .. }
            | Self::Tools
            | Self::StreamOptions => Some(400),
            Self::Reply { .. } | Self::MessageShape { .. } => Some(500),
            Self::ModelListCredentials
            | Self::AccountId
            | Self::ModelListStatus { .. }
            | Self::ModelsDecode { .. }
            | Self::ModelsTooLarge => None,
        }
    }
}
