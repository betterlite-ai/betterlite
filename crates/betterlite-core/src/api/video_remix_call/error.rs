use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::config::VideoOperation;
use super::shared::MockDecodeError;
use crate::api::video_remix_list_transport::error::{FieldError, VideoRemixListTransportError};
use crate::api::video_selection::error::VideoSelectionError;

// exception_type() maps every video failure with model "" and the provider the call reached
const MODEL: &str = "";

// what video_remix, avideo_remix, video_list and avideo_list raise before exception_type sees it
#[derive(Debug, Error)]
pub enum VideoCallError {
    #[error(transparent)]
    Mock(#[from] MockDecodeError),
    // LlmProviders(custom_llm_provider) rejects a name outside the provider set
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    // the body's ValueError
    #[error("video {operation} is not supported for {provider}")]
    Unsupported {
        operation: VideoOperation,
        provider: String,
    },
    // the provider has a config in LiteLLM, but no adapter for it is installed here yet
    #[error("no video config is installed for '{provider}'")]
    ConfigMissing { provider: String },
    // B3332's own message for a provider whose config class is not built yet
    #[error(transparent)]
    Selection(#[from] VideoSelectionError),
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Params(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // avideo_list's provider inference, which runs before the mock short circuit
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Transport(#[from] VideoRemixListTransportError),
    #[error(
        "the video {operation} handler answered with a future where a direct answer was expected"
    )]
    UnexpectedPending { operation: VideoOperation },
}

// the failures a sync handler's future turns into
pub(crate) const LIST_PENDING: VideoCallError = VideoCallError::UnexpectedPending {
    operation: VideoOperation::List,
};
pub(crate) const REMIX_PENDING: VideoCallError = VideoCallError::UnexpectedPending {
    operation: VideoOperation::Remix,
};

#[derive(Debug, Error)]
#[error("{error}")]
pub struct VideoCallFailure {
    #[source]
    pub error: VideoCallError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...), run once
#[must_use]
pub fn failure(error: VideoCallError, provider: &str, redactor: &Redactor) -> VideoCallFailure {
    let class = match &error {
        VideoCallError::Transport(transport) => transport_class(transport, provider, redactor),
        VideoCallError::Resolution(resolution) => {
            // avideo_list's get_llm_provider failure is a BadRequestError that exception_type returns unchanged
            dispatch(
                &DispatchInput::Raw(Box::new(exception(
                    resolution.to_string(),
                    provider,
                    Some(400),
                    false,
                ))),
                redactor,
            )
        }
        VideoCallError::Mock(_)
        | VideoCallError::UnknownProvider { .. }
        | VideoCallError::Unsupported { .. }
        | VideoCallError::ConfigMissing { .. }
        | VideoCallError::Selection(_)
        | VideoCallError::Params(_)
        | VideoCallError::Timeout { .. }
        | VideoCallError::Logging(_)
        | VideoCallError::UnexpectedPending { .. } => {
            value_error(error.to_string(), provider, redactor)
        }
    };
    VideoCallFailure { error, class }
}

// a provider answer keeps its status, and every other transport failure is the catchall or a named field
fn transport_class(
    error: &VideoRemixListTransportError,
    provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    match error {
        // the handler's _handle_error answer, with the status httpx read off the response
        VideoRemixListTransportError::Provider(provider_error) => dispatch(
            &DispatchInput::Raw(Box::new(exception(
                provider_error.message.clone(),
                provider,
                Some(provider_error.status_code),
                provider_error.status_code_is_synthesized,
            ))),
            redactor,
        ),
        VideoRemixListTransportError::Field(FieldError::Type { field, .. }) => {
            named_field_class(error.to_string(), field, provider)
        }
        VideoRemixListTransportError::UnknownProvider { .. }
        | VideoRemixListTransportError::Client { .. }
        | VideoRemixListTransportError::Prepare { .. }
        | VideoRemixListTransportError::Logging(_)
        | VideoRemixListTransportError::RunningRuntime { .. }
        | VideoRemixListTransportError::Runtime { .. } => {
            value_error(error.to_string(), provider, redactor)
        }
    }
}

fn value_error(message: String, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception(message, provider, None, false);
    "ValueError".clone_into(&mut raw.exception_type);
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// docs/compatibility.md rejects a value outside its declared type with a 400 that names the field
fn named_field_class(message: String, field: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message,
        model: Some(MODEL.to_owned()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

fn exception(
    message: String,
    provider: &str,
    status_code: Option<u16>,
    status_code_is_synthesized: bool,
) -> RawException {
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
