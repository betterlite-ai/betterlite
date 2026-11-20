use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::image_variations::error::{ImageVariationSource, OpenAiImageVariationError};
use betterlite_redact::secret::Redactor;
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
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct TopazVariationError {
    pub status_code: Option<u16>,
    pub message: String,
}

// aimage_variation's failure, after litellm's exception_type has mapped the sync exception
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariationException {
    pub class: ErrorClass,
    pub message: String,
}

impl VariationException {
    pub(crate) fn mapped(
        error: &ImageVariationError,
        model: Option<&str>,
        provider: &str,
        redactor: &Redactor,
    ) -> Self {
        match error {
            ImageVariationError::Resolution(source) => {
                let message = source.to_string();
                project(
                    raw_exception(&message, Some(400), model, provider),
                    &message,
                    redactor,
                )
            }
            ImageVariationError::OpenAi(source) => {
                let error_str = openai_projection(source);
                project(
                    raw_exception(&error_str, Some(source.status_code), model, provider),
                    &source.message,
                    redactor,
                )
            }
            ImageVariationError::Topaz(source) => project(
                raw_exception(&source.message, source.status_code, model, provider),
                &source.message,
                redactor,
            ),
            ImageVariationError::InvalidProvider { .. }
            | ImageVariationError::UnknownModelInfo { .. }
            | ImageVariationError::MissingKey { .. }
            | ImageVariationError::MissingBase { .. }
            | ImageVariationError::InvalidTimeout { .. } => {
                let message = error.to_string();
                project(
                    raw_exception(&message, None, model, provider),
                    &message,
                    redactor,
                )
            }
        }
    }
}

// the handler wraps the SDK failure in OpenAIError(status_code, message, headers), whose only text is this
fn openai_projection(source: &OpenAiImageVariationError) -> String {
    match source.source.as_ref() {
        ImageVariationSource::Status { .. } => {
            format!("Error code: {} - {}", source.status_code, source.message)
        }
        ImageVariationSource::MissingConfig { .. }
        | ImageVariationSource::MissingData
        | ImageVariationSource::MaxRetries
        | ImageVariationSource::MissingApiKey
        | ImageVariationSource::ClientKind { .. }
        | ImageVariationSource::Option { .. }
        | ImageVariationSource::Unsupported { .. }
        | ImageVariationSource::Form(_)
        | ImageVariationSource::Url(_)
        | ImageVariationSource::Transport(_)
        | ImageVariationSource::Logging(_)
        | ImageVariationSource::Reply(_)
        | ImageVariationSource::Header(_) => source.message.clone(),
    }
}

// the attributes exception_type reads off the original exception before any provider branch runs
pub(crate) fn raw_exception(
    error_str: &str,
    status_code: Option<u16>,
    model: Option<&str>,
    provider: &str,
) -> RawException {
    RawException {
        model: model.unwrap_or_default().to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(error_str.to_owned()),
        body: None,
        exception_type: String::new(),
    }
}

// exception_type's full dispatch: the timeout preamble, every provider family, then _map_exception_by_status
pub(crate) fn project(raw: RawException, message: &str, redactor: &Redactor) -> VariationException {
    VariationException {
        class: dispatch(&DispatchInput::Raw(Box::new(raw)), redactor),
        message: message.to_owned(),
    }
}
