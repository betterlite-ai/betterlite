use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::image_models::sizing::SizeError;

// B682's BedrockError implements no Display or Error, so the derived wrapper carries the cause instead.
#[derive(Debug, Error)]
#[error("bedrock provider error")]
pub struct BedrockFailure {
    pub cause: Box<BedrockError>,
}

#[derive(Debug, Error)]
pub enum PegasusError {
    #[error("the response body is not valid JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response body is not a JSON object")]
    ReplyNotObject,
    #[error("the response message field is not a string")]
    MessageNotString,
    #[error("the response finishReason field cannot be mapped")]
    FinishReasonNotMappable,
    #[error("the response content could not be set")]
    ContentNotSet,
    #[error("the response token count header {name} is not an integer")]
    TokenCountHeader {
        name: &'static str,
        #[source]
        source: SizeError,
    },
    #[error("the token count failed")]
    TokenCount {
        #[source]
        source: CountError,
    },
    #[error("the total token count overflows a signed 64-bit integer")]
    TotalOverflow,
    #[error("the response creation time is out of range")]
    CreatedOutOfRange,
    #[error("bedrock provider error ({status})")]
    Bedrock {
        status: u16,
        #[source]
        error: BedrockFailure,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("response_format must be a JSON object or null")]
    UnsupportedResponseFormat,
    #[error("the {field} field must be a string")]
    NotText { field: &'static str },
}
