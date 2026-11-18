use std::fmt;

use betterlite_error::error::{ErrorClass, ErrorMetadata};
use betterlite_error::openai::{OpenaiExceptionInput, map_openai_exception};
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::image_variations::error::OpenAiImageVariationError;
use thiserror::Error;

// image_variation's own failures: every message is the one its ValueError carries in LiteLLM
#[derive(Debug, Error)]
pub enum ImageVariationError {
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(
        "Invalid image variation provider: {provider}. Supported providers are: <enum 'LITELLM_IMAGE_VARIATION_PROVIDERS'>"
    )]
    InvalidProvider { provider: String },
    #[error(
        "image variation provider has no known model info config - required for getting api keys, etc.: {provider}. Supported providers are: <enum 'LITELLM_IMAGE_VARIATION_PROVIDERS'>"
    )]
    UnknownModelInfo { provider: String },
    #[error("API key is required for {provider} image variations")]
    MissingKey { provider: &'static str },
    #[error("API base is required for {provider} image variations")]
    MissingBase { provider: &'static str },
    #[error("timeout must be a non-negative number of seconds, got {seconds}")]
    InvalidTimeout { seconds: f64 },
    #[error(transparent)]
    OpenAi(#[from] OpenAiImageVariationError),
    #[error(transparent)]
    Topaz(#[from] TopazVariationError),
}

// what a Topaz executor reports, mirroring the TopazException its aiohttp handler raises
#[derive(Debug, Error)]
#[error("{message}")]
pub struct TopazVariationError {
    pub status_code: Option<u16>,
    pub message: String,
}

// aimage_variation's failure, after litellm's exception_type has mapped the sync exception
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariationException {
    class: ErrorClass,
    pub message: String,
}

impl VariationException {
    pub(crate) fn mapped(error: &ImageVariationError, model: Option<&str>, provider: &str) -> Self {
        let (status, message) = match error {
            ImageVariationError::Resolution(source) => (Some(400), source.to_string()),
            ImageVariationError::OpenAi(source) => {
                (Some(source.status_code), source.message.clone())
            }
            ImageVariationError::Topaz(source) => (source.status_code, source.message.clone()),
            ImageVariationError::InvalidProvider { .. }
            | ImageVariationError::UnknownModelInfo { .. }
            | ImageVariationError::MissingKey { .. }
            | ImageVariationError::MissingBase { .. }
            | ImageVariationError::InvalidTimeout { .. } => (None, error.to_string()),
        };
        project(&message, status, model, provider)
    }

    // ImageResponse(**cached_dict) can fail the way pydantic does, and no status reaches that mapping
    pub(crate) fn unmapped(
        source: &serde_json::Error,
        model: Option<&str>,
        provider: &str,
    ) -> Self {
        project(&source.to_string(), None, model, provider)
    }

    #[must_use]
    pub fn status_code(&self) -> u16 {
        self.class.metadata().status_code
    }

    #[must_use]
    pub fn metadata(&self) -> ErrorMetadata {
        self.class.metadata()
    }

    #[must_use]
    pub fn class(&self) -> &ErrorClass {
        &self.class
    }
}

impl fmt::Display for VariationException {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for VariationException {}

// litellm's exception_type reads no structured body off these exceptions, so only status and text reach it
fn project(
    message: &str,
    status_code: Option<u16>,
    model: Option<&str>,
    provider: &str,
) -> VariationException {
    let input = OpenaiExceptionInput {
        model: model.unwrap_or_default().to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message.to_owned()),
        body: None,
    };
    VariationException {
        class: map_openai_exception(&input),
        message: message.to_owned(),
    }
}
