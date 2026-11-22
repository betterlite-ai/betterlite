use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::controls::multipart::FormFieldError;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::image_input_options::selection::ImageInputOptionsError;

// image_edit's own failures: every message is the one its ValueError or handler error carries in LiteLLM
#[derive(Debug, Error)]
pub enum ImageEditError {
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error("image edit is not supported for {provider}")]
    UnsupportedConfig { provider: String },
    #[error(
        "Unmapped LLM provider for this endpoint. You passed model={model}, custom_llm_provider={provider}. Check supported provider and route: https://docs.litellm.ai/docs/providers"
    )]
    UnknownProvider { model: String, provider: String },
    #[error(transparent)]
    Options(#[from] ImageInputOptionsError),
    #[error(transparent)]
    FormField(#[from] FormFieldError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the {name} header value must be text")]
    HeaderText { name: String },
    #[error("timeout must be a non-negative number of seconds, got {seconds}")]
    InvalidTimeout { seconds: f64 },
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Provider(#[from] ProviderFailure),
    #[error("the image edit transport failed: {message}")]
    Transport { message: String },
    #[error(transparent)]
    Logging(#[from] PrePostError),
}

// what a provider send reports: the status, the reply text, and whether the handler invented the status
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct ProviderFailure {
    pub status_code: u16,
    pub message: String,
    pub status_code_is_synthesized: bool,
}

// aimage_edit's failure, after litellm's exception_type has mapped the sync exception
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEditException {
    pub class: ErrorClass,
    pub message: String,
}

impl ImageEditException {
    #[must_use]
    pub fn mapped(
        error: &ImageEditError,
        model: Option<&str>,
        provider: &str,
        redactor: &Redactor,
    ) -> Self {
        match error {
            // Invalid is a request value the declared type can't hold: docs/compatibility.md's 400 that names the field
            ImageEditError::Invalid(_)
            | ImageEditError::Resolution(_)
            | ImageEditError::UnknownProvider { .. } => project(
                raw_exception(&error.to_string(), Some(400), false, model, provider),
                redactor,
            ),
            // str() of the provider config's BaseLLMException is the reply text itself
            ImageEditError::Provider(source) => project(
                raw_exception(
                    &source.message,
                    Some(source.status_code),
                    source.status_code_is_synthesized,
                    model,
                    provider,
                ),
                redactor,
            ),
            // LiteLLM raises UnsupportedParamsError itself and exception_type passes it through: 400, no type, param null, no code
            ImageEditError::Options(ImageInputOptionsError::UnsupportedParams { .. }) => {
                Self::unsupported_params(error, model, provider)
            }
            ImageEditError::UnsupportedConfig { .. }
            | ImageEditError::FormField(_)
            | ImageEditError::Json(_)
            | ImageEditError::HeaderText { .. }
            | ImageEditError::InvalidTimeout { .. }
            | ImageEditError::Transport { .. }
            | ImageEditError::Logging(_) => project(
                raw_exception(&error.to_string(), None, false, model, provider),
                redactor,
            ),
        }
    }

    fn unsupported_params(error: &ImageEditError, model: Option<&str>, provider: &str) -> Self {
        Self {
            class: ErrorClass::UnsupportedParams(Box::new(ErrorContext {
                message: error.to_string(),
                model: model.map(str::to_owned),
                provider: Some(provider.to_owned()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            })),
            message: error.to_string(),
        }
    }
}

// the attributes exception_type reads off the original exception before any provider branch runs
#[must_use]
pub(crate) fn raw_exception(
    error_str: &str,
    status_code: Option<u16>,
    status_code_is_synthesized: bool,
    model: Option<&str>,
    provider: &str,
) -> RawException {
    RawException {
        model: model.unwrap_or_default().to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        status_code_is_synthesized,
        message: Some(error_str.to_owned()),
        body: None,
        exception_type: String::new(),
    }
}

// exception_type's full dispatch: the timeout preamble, every provider family, then _map_exception_by_status
#[must_use]
pub(crate) fn project(raw: RawException, redactor: &Redactor) -> ImageEditException {
    let message = raw.error_str.clone();
    ImageEditException {
        class: dispatch(&DispatchInput::Raw(Box::new(raw)), redactor),
        message,
    }
}
