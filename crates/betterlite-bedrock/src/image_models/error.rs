use thiserror::Error;

use crate::bedrock_settings::{BedrockError, HeaderError};

use super::sizing::SizeError;

#[derive(Debug, Error)]
pub enum ImageModelsError {
    #[error(transparent)]
    Size(#[from] SizeError),
    #[error("got multiple values for keyword argument '{0}'")]
    KeywordCollision(&'static str),
    #[error("Task type {0} is not supported")]
    TaskTypeUnsupported(String),
    #[error("'{0}' is not a mapping")]
    NotAMapping(&'static str),
    #[error("sequence item {0}: expected str instance, {1} found")]
    JoinNotAString(usize, &'static str),
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}
