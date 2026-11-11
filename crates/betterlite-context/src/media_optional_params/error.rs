use crate::non_default_params::error::{MapperError, NonDefaultParamsError};

#[derive(Debug, thiserror::Error)]
pub enum MediaParamsError {
    #[error(
        "litellm.UnsupportedParamsError: Setting user/encoding format is not supported by {provider}. To drop it from the call, set `litellm.drop_params = True`."
    )]
    TranscriptionUnsupported { provider: String },
    #[error(
        "litellm.UnsupportedParamsError: Setting `{param}` is not supported by {provider}, {model}. To drop it from the call, set `litellm.drop_params = True`."
    )]
    ImageUnsupported {
        param: String,
        provider: String,
        model: String,
    },
    #[error("bedrock image generation needs the config class of the model")]
    MissingBedrockConfig,
    #[error("{field} must be {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Params(#[from] NonDefaultParamsError),
    #[error(transparent)]
    Mapper(#[from] MapperError),
}

impl MediaParamsError {
    // LiteLLM's UnsupportedParamsError ignores the status it is given and always carries 400.
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Params(error) => error.status_code(),
            Self::Mapper(error) => error.status_code,
            Self::TranscriptionUnsupported { .. }
            | Self::ImageUnsupported { .. }
            | Self::MissingBedrockConfig
            | Self::WrongType { .. } => 400,
        }
    }
}
