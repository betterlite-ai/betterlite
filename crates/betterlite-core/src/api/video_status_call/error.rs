use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::video_selection::error::VideoSelectionError;
use crate::api::video_status_transport::error::VideoStatusError;

#[derive(Debug, Error)]
pub enum VideoStatusCallError {
    // LiteLLM's ValueError, which exception_type turns into an APIConnectionError
    #[error("video status is not supported for {provider}")]
    Unsupported { provider: String },
    #[error(transparent)]
    Selection(#[from] VideoSelectionError),
    // litellm.LlmProviders(custom_llm_provider) refused the name
    #[error("'{provider}' is not a valid LlmProviders")]
    UnknownProvider {
        provider: String,
        #[source]
        source: strum::ParseError,
    },
    #[error("{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // json.loads(mock_response) on a string, and VideoObject(**mock_response) after it
    #[error("the mock response is not valid JSON")]
    MockJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match a video object")]
    MockModel {
        #[source]
        source: serde_json::Error,
    },
    #[error("the blocking status call waited on a handler that never answered")]
    Suspended,
    // the error exception_type produced, kept so the async wrapper cannot map it a second time
    #[error("{message}")]
    Mapped { class: ErrorClass, message: String },
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...); a mapped error is returned as it is
#[must_use]
pub fn failure(
    error: VideoStatusCallError,
    provider: &str,
    redactor: &Redactor,
) -> VideoStatusCallError {
    if let VideoStatusCallError::Mapped {
        class,
        message: mapped,
    } = error
    {
        return remapped(class, mapped, redactor);
    }
    rejected_failure(&error, provider, redactor)
}

// dispatch wraps the class a mapped error already carries, leaving its message as it was
fn remapped(class: ErrorClass, message: String, redactor: &Redactor) -> VideoStatusCallError {
    VideoStatusCallError::Mapped {
        class: dispatch(&DispatchInput::Mapped(class), redactor),
        message,
    }
}

fn rejected_failure(
    error: &VideoStatusCallError,
    provider: &str,
    redactor: &Redactor,
) -> VideoStatusCallError {
    if let VideoStatusCallError::FieldType { field, expected } = error {
        return field_failure(field, expected, provider, error.to_string());
    }
    let message = error.to_string();
    let raw = rejected_exception(&message, provider);
    raw_failure(message, raw, redactor)
}

// what the B3337 handler's failure carries into exception_type: a converted class or an unconverted provider failure
#[must_use]
pub fn status_failure(
    error: &VideoStatusError,
    provider: &str,
    redactor: &Redactor,
) -> VideoStatusCallError {
    let raw = match error {
        VideoStatusError::Converted(carried) => {
            let mut raw =
                raw_exception(carried.message.clone(), provider, Some(carried.status_code));
            raw.status_code_is_synthesized = carried.status_code_is_synthesized;
            raw
        }
        VideoStatusError::Unconverted(carried) => {
            raw_exception(carried.message.clone(), provider, carried.status_code)
        }
        VideoStatusError::FieldType { field, expected } => {
            return field_failure(field, expected, provider, error.to_string());
        }
        VideoStatusError::Client(_)
        | VideoStatusError::Logging(_)
        | VideoStatusError::UnknownProvider { .. }
        | VideoStatusError::Suspended => raw_exception(error.to_string(), provider, None),
    };
    raw_failure(error.to_string(), raw, redactor)
}

fn field_failure(
    field: &str,
    expected: &str,
    provider: &str,
    message: String,
) -> VideoStatusCallError {
    VideoStatusCallError::Mapped {
        class: field_class(field, expected, provider),
        message,
    }
}

fn raw_failure(message: String, raw: RawException, redactor: &Redactor) -> VideoStatusCallError {
    VideoStatusCallError::Mapped {
        class: dispatch(&DispatchInput::Raw(Box::new(raw)), redactor),
        message,
    }
}

// the ValueError raised outside a provider answer, with type(original_exception).__name__ set
fn rejected_exception(message: &str, provider: &str) -> RawException {
    let mut exception = raw_exception(message.to_owned(), provider, None);
    "ValueError".clone_into(&mut exception.exception_type);
    exception
}

fn raw_exception(message: String, provider: &str, status_code: Option<u16>) -> RawException {
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}

// docs/compatibility.md: a value the declared type cannot hold is a 400 that names the field
fn field_class(field: &str, expected: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message: format!("{field} must be {expected}"),
        model: Some(String::new()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}
