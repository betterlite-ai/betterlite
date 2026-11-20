use thiserror::Error;

use crate::api::messages_chat_content::error::ChatContentError;
use crate::api::messages_chat_options::error::OptionsError;
use crate::api::messages_chat_tools_schema::output_format::OutputFormatError;
use crate::api::messages_chat_tools_schema::tool_choice::ToolChoiceError;
use crate::api::messages_chat_tools_schema::tools::ToolsError;

// LiteLLM raises KeyError for a popped required key and ValueError for an empty one, so the wrapper keeps that split.
#[derive(Debug, Error)]
pub enum TransformError {
    // Python spells str(KeyError("model")) with the quotes, and both wrappers surface it unchanged.
    #[error("'{key}'")]
    MissingKey { key: &'static str },
    #[error("Bad Request: model is required for Anthropic Messages Request")]
    ModelRequired,
    #[error("Bad Request: messages is required for Anthropic Messages Request")]
    MessagesRequired,
    #[error("model must be a string")]
    ModelNotText,
    #[error("messages must be a list")]
    MessagesNotList,
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error(transparent)]
    Options(#[from] OptionsError),
    #[error(transparent)]
    ToolChoice(#[from] ToolChoiceError),
    #[error(transparent)]
    Tools(#[from] ToolsError),
    #[error(transparent)]
    OutputFormat(#[from] OutputFormatError),
}
