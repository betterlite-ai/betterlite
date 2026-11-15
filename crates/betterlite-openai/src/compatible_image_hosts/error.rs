use thiserror::Error;

use crate::error::ProviderError;

#[derive(Debug, Error)]
pub enum ImageOptionError {
    #[error("parameter {name} is not supported for model {model}")]
    Unsupported { name: String, model: String },
    #[error("image size {size:?} is not two integers around an x")]
    Size { size: String },
    #[error("the AIML request already names {name}")]
    Duplicate { name: String },
}

#[derive(Debug, Error)]
pub enum ImageCredentialError {
    #[error("AIML_API_KEY or AIMLAPI_KEY is not set")]
    MissingAimlKey,
    #[error("COMETAPI_KEY or COMETAPI_API_KEY is not set")]
    MissingCometApiKey,
}

// the JSON failure is B997's error carrying the reply's own status and headers, while a shape failure keeps this class
#[derive(Debug, Error)]
pub enum CompatibleImageReplyError {
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error("the image reply is {bytes} bytes, over the {limit} byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the image reply is not an object")]
    NotObject,
    #[error("{field} must be an object")]
    Object { field: &'static str },
    #[error("{field} must be a list")]
    List { field: &'static str },
    #[error("{field} must be text or null")]
    Text { field: &'static str },
}

impl CompatibleImageReplyError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Provider(error) => Some(error.status_code),
            Self::TooLarge { .. }
            | Self::NotObject
            | Self::Object { .. }
            | Self::List { .. }
            | Self::Text { .. } => None,
        }
    }
}
