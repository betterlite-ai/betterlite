use betterlite_openai::image_generation::family::ImageParamError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FoundryImageError {
    #[error("api_base is required for Azure AI FLUX 2 image generation")]
    ApiBaseRequired,
    #[error("Invalid size format '{size}'. Expected 'WxH', for example '1024x1024'.")]
    InvalidSize { size: String },
    #[error(
        "Parameters {names:?} are not supported for model {model}. Supported parameters are {supported:?}. Set drop_params=True to drop unsupported parameters."
    )]
    UnsupportedParams {
        model: String,
        names: Vec<String>,
        supported: &'static [&'static str],
    },
    #[error("parameter {field} must be a whole number, got {value}")]
    WholeNumber { field: String, value: String },
    #[error("parameter {field} must be a number, got {value}")]
    Decimal { field: String, value: String },
    #[error(transparent)]
    GptParams(#[from] ImageParamError),
}

impl FoundryImageError {
    // LiteLLM raises BadRequestError for a bad size and UnsupportedParamsError for unknown parameters, both 400 with provider azure_ai; a failed conversion or the inherited GPT policy is a ValueError with no status
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::InvalidSize { .. } | Self::UnsupportedParams { .. } => Some(400),
            Self::ApiBaseRequired
            | Self::WholeNumber { .. }
            | Self::Decimal { .. }
            | Self::GptParams(_) => None,
        }
    }

    #[must_use]
    pub const fn llm_provider(&self) -> Option<&'static str> {
        match self.status_code() {
            Some(_) => Some("azure_ai"),
            None => None,
        }
    }
}
