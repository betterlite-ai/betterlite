use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::error::ProviderError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::video_edit_transport::error::{VideoClientError, VideoEditTransportError};
use crate::api::video_selection::error::VideoSelectionError;

// exception_type() maps every video edit failure with model ""
const MODEL: &str = "";

// LiteLLM's mock parse, then VideoObject(**mock_response), whose failures are ValueErrors too
#[derive(Debug, Error)]
pub enum MockDecodeError {
    #[error("the mock response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match a video object")]
    Model {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Error)]
pub enum VideoEditCallError {
    #[error("the litellm_logging_obj keyword is required")]
    MissingLoggingObject,
    #[error("the video edit handler answered with a pending future on its sync path")]
    SyncPending,
    #[error(transparent)]
    Mock(#[from] MockDecodeError),
    // litellm.LlmProviders(custom_llm_provider) for a name betterlite's provider enum does not know
    #[error(transparent)]
    UnknownProvider(#[from] VideoClientError),
    #[error("video edit is not supported for {provider}")]
    ProviderConfigMissing { provider: String },
    #[error(transparent)]
    Selection(#[from] VideoSelectionError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(#[from] VideoEditTransportError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct VideoEditFailure {
    #[source]
    pub error: VideoEditCallError,
    pub class: ErrorClass,
}

impl VideoEditFailure {
    // the async wrapper's except runs exception_type again, and exception_type returns an already mapped exception unchanged
    #[must_use]
    pub fn remapped(self, redactor: &Redactor) -> Self {
        Self {
            class: dispatch(&DispatchInput::Mapped(self.class), redactor),
            error: self.error,
        }
    }
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(error: VideoEditCallError, provider: &str, redactor: &Redactor) -> VideoEditFailure {
    let class = match &error {
        VideoEditCallError::Handler(VideoEditTransportError::Provider(provider_error)) => {
            provider_class(provider_error, provider, redactor)
        }
        VideoEditCallError::Handler(handler_error) => {
            plain(&handler_error.to_string(), provider, "ValueError", redactor)
        }
        VideoEditCallError::MissingLoggingObject => {
            plain(&error.to_string(), provider, "KeyError", redactor)
        }
        VideoEditCallError::SyncPending
        | VideoEditCallError::Mock(_)
        | VideoEditCallError::UnknownProvider(_)
        | VideoEditCallError::ProviderConfigMissing { .. }
        | VideoEditCallError::Selection(_)
        | VideoEditCallError::Logging(_) => {
            plain(&error.to_string(), provider, "ValueError", redactor)
        }
    };
    VideoEditFailure { error, class }
}

// get_error_class's BaseLLMException is raised by the handler, so exception_type sees its message and status
fn provider_class(error: &ProviderError, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception(
        &error.message,
        provider,
        Some(error.status_code),
        "BaseLLMException",
    );
    raw.status_code_is_synthesized = error.status_code_is_synthesized;
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

fn plain(message: &str, provider: &str, exception_type: &str, redactor: &Redactor) -> ErrorClass {
    dispatch(
        &DispatchInput::Raw(Box::new(exception(message, provider, None, exception_type))),
        redactor,
    )
}

fn exception(
    message: &str,
    provider: &str,
    status_code: Option<u16>,
    exception_type: &str,
) -> RawException {
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message.to_owned()),
        body: None,
        exception_type: exception_type.to_owned(),
    }
}
