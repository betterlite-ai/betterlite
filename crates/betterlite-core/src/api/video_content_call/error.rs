use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::video_selection::error::VideoSelectionError;
use crate::api::videos_http_transport::error::VideoTransportError;
use crate::api::videos_http_transport::video_config::VideoHttpConfig;

#[derive(Debug, Error)]
pub enum VideoContentCallError {
    // LiteLLM's ValueError, which exception_type turns into an APIConnectionError
    #[error("video support download is not supported for {provider}")]
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
    #[error("the blocking content call waited on a handler that never answered")]
    Suspended,
    // the error exception_type produced, kept so the async wrapper cannot map it a second time
    #[error("{message}")]
    Mapped { class: ErrorClass, message: String },
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...); a mapped error is returned as it is
#[must_use]
pub fn failure(
    error: VideoContentCallError,
    provider: &str,
    redactor: &Redactor,
) -> VideoContentCallError {
    let message = error.to_string();
    match error {
        VideoContentCallError::Mapped {
            class,
            message: mapped,
        } => VideoContentCallError::Mapped {
            class: dispatch(&DispatchInput::Mapped(class), redactor),
            message: mapped,
        },
        VideoContentCallError::FieldType { field, expected } => VideoContentCallError::Mapped {
            class: field_class(field, expected, provider),
            message,
        },
        rejected => VideoContentCallError::Mapped {
            class: dispatch(
                &DispatchInput::Raw(Box::new(rejected_exception(&message, provider))),
                redactor,
            ),
            message,
        },
    }
}

// what the B3333 handler's failure carries into exception_type: the provider's own status, or the local text
#[must_use]
pub fn transport_failure<Config>(
    error: VideoTransportError<Config::Error>,
    provider: &str,
    config: &Config,
    redactor: &Redactor,
) -> VideoContentCallError
where
    Config: VideoHttpConfig,
{
    let message = error.to_string();
    let (text, status_code, synthesized) = match &error {
        VideoTransportError::Provider(carried) => (
            carried.message.clone(),
            Some(carried.status_code),
            carried.status_code_is_synthesized,
        ),
        VideoTransportError::Config(source) => {
            let facts = config.error_facts(source);
            (facts.message, facts.status_code, false)
        }
        VideoTransportError::Client(_) | VideoTransportError::Logging(_) => {
            (message.clone(), None, false)
        }
        VideoTransportError::Field(field) => {
            return VideoContentCallError::Mapped {
                class: field_class(field.field, field.expected, provider),
                message,
            };
        }
    };
    let mut raw = raw_exception(text, provider, status_code);
    raw.status_code_is_synthesized = synthesized;
    VideoContentCallError::Mapped {
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
