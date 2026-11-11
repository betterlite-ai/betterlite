use crate::non_default_params::error::{MapperError, NonDefaultParamsError};

#[derive(Debug, thiserror::Error)]
pub enum MediaParamsError {
    #[error("setting {param} is not supported by {provider}")]
    TranscriptionUnsupported { param: String, provider: String },
    #[error("{param} is not supported by {provider} for {model}")]
    ImageUnsupported {
        param: String,
        provider: String,
        model: String,
    },
    #[error("bedrock image generation needs the config class of the model")]
    MissingBedrockConfig,
    #[error("{0} must be {1}")]
    WrongType(&'static str, &'static str),
    #[error(transparent)]
    Params(#[from] NonDefaultParamsError),
    #[error(transparent)]
    Mapper(#[from] MapperError),
}

impl MediaParamsError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Params(error) => error.status_code(),
            Self::Mapper(error) => error.status_code,
            Self::TranscriptionUnsupported { .. }
            | Self::ImageUnsupported { .. }
            | Self::MissingBedrockConfig
            | Self::WrongType(..) => 400,
        }
    }
}
