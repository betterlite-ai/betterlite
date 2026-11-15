use betterlite_prompts::arguments::json::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::gemini_format::response::GeminiFormatError;
use crate::gemini_thinking::error::GeminiThinkingError;
use crate::gemini_tools::error::GeminiToolsError;

#[derive(Debug, Error)]
pub enum GeminiOptionsError {
    #[error(transparent)]
    Tools(#[from] GeminiToolsError),
    #[error(transparent)]
    Thinking(#[from] GeminiThinkingError),
    #[error(transparent)]
    Format(#[from] GeminiFormatError),
    #[error("`{param}` must be {expected}")]
    FieldType {
        param: String,
        expected: &'static str,
    },
    #[error("`{param}` is not valid")]
    Invalid {
        param: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("`{param}` must be a list")]
    NotList { param: String },
    #[error("`{field}` is not a finite number")]
    NonFinite { field: String },
    #[error("a mapped value cannot be encoded")]
    Encode(#[source] serde_json::Error),
}

impl GeminiOptionsError {
    // LiteLLM raises UnsupportedParamsError (400) for an unsupported tool choice, and everything else reaches the caller as a 500
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Thinking(error) => error.status(),
            Self::Tools(GeminiToolsError::UnsupportedToolChoice)
            | Self::FieldType { .. }
            | Self::Invalid { .. } => Some(400),
            Self::Tools(_)
            | Self::Format(_)
            | Self::NotList { .. }
            | Self::NonFinite { .. }
            | Self::Encode(_) => Some(500),
        }
    }
}

pub fn encode<T>(value: T) -> Result<Json, GeminiOptionsError>
where
    T: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).map_err(GeminiOptionsError::Encode)?)
        .map_err(GeminiOptionsError::Encode)
}

pub fn decode<T>(value: &Json, param: &str) -> Result<T, GeminiOptionsError>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_value(value).map_err(GeminiOptionsError::Encode)?;
    serde_json::from_value(encoded).map_err(|source| GeminiOptionsError::Invalid {
        param: param.to_owned(),
        source,
    })
}
