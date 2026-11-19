use std::time::TryFromFloatSecsError;

use betterlite_azure::azure_message_attachments::error::MessagesError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_openai::threads::error::ThreadsError;
use betterlite_prompts::arguments::json::Json;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::TimeoutError;

use super::options::OptionError;

// everything the wrapper resolves before it reaches a provider client
#[derive(Debug, Error)]
pub enum SetupError {
    #[error(transparent)]
    Options(#[from] OptionError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("the selected message is not a valid SDK message: {source}")]
    Data {
        #[source]
        source: serde_json::Error,
    },
    #[error("the timeout {seconds} seconds is not a duration")]
    Deadline {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("LiteLLM doesn't support {provider} for '{operation}'. Only 'openai' is supported.")]
    UnsupportedProvider {
        provider: String,
        operation: &'static str,
    },
}

#[derive(Debug, Error)]
pub enum MessageError {
    #[error(transparent)]
    Setup(#[from] SetupError),
    #[error(transparent)]
    OpenAi(#[from] ThreadsError),
    #[error(transparent)]
    Azure(#[from] MessagesError),
    #[error("the provider answered the {operation} call with the wrong SDK object")]
    UnexpectedReply { operation: &'static str },
}

impl MessageError {
    // the HTTP status a provider answer carried, which the raw SDK exception reports as-is
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::OpenAi(ThreadsError::Status { status, .. }) => Some(*status),
            Self::Azure(MessagesError::Threads(ThreadsError::Status { status, .. })) => {
                Some(*status)
            }
            Self::Setup(_) | Self::OpenAi(_) | Self::Azure(_) | Self::UnexpectedReply { .. } => {
                None
            }
        }
    }

    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        match self {
            Self::OpenAi(ThreadsError::Status { body, .. }) => Some(body),
            Self::Azure(MessagesError::Threads(ThreadsError::Status { body, .. })) => Some(body),
            Self::Setup(_) | Self::OpenAi(_) | Self::Azure(_) | Self::UnexpectedReply { .. } => {
                None
            }
        }
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct MessageFailure {
    #[source]
    pub error: MessageError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(error: MessageError, provider: &str, redactor: &Redactor) -> MessageFailure {
    let class = match &error {
        MessageError::Setup(setup) => setup_class(setup, provider),
        MessageError::OpenAi(_) | MessageError::Azure(_) => dispatch(
            &DispatchInput::Raw(Box::new(exception(&error, provider))),
            redactor,
        ),
        MessageError::UnexpectedReply { .. } => {
            ErrorClass::Unclassified(Box::new(context(error.to_string(), provider)))
        }
    };
    MessageFailure { error, class }
}

// the selector's own classes never reach the provider, so they map straight from its error
fn setup_class(error: &SetupError, provider: &str) -> ErrorClass {
    match error {
        SetupError::Options(options) if options.is_unsupported_params() => {
            ErrorClass::UnsupportedParams(Box::new(context(options.python_message(), provider)))
        }
        SetupError::UnsupportedProvider { .. } => {
            let details = ErrorContext {
                model: Some("n/a".to_owned()),
                ..context(error.to_string(), provider)
            };
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(details, None)))
        }
        SetupError::Options(_)
        | SetupError::Timeout(_)
        | SetupError::Data { .. }
        | SetupError::Deadline { .. } => {
            ErrorClass::Unclassified(Box::new(context(error.to_string(), provider)))
        }
    }
}

fn context(message: String, provider: &str) -> ErrorContext {
    ErrorContext {
        message,
        model: None,
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

// the openai SDK's APIStatusError: str(e) is "Error code: {status} - {body}" and e.body is the error object
fn exception(error: &MessageError, provider: &str) -> RawException {
    let status = error.status();
    let body = error.body();
    let error_str = status.map_or_else(
        || error.to_string(),
        |status| {
            format!(
                "Error code: {status} - {}",
                body.map_or("", |bytes| std::str::from_utf8(bytes).unwrap_or(""))
            )
        },
    );
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str,
        status_code: status,
        status_code_is_synthesized: false,
        message: None,
        body: body.and_then(flattened_body),
        exception_type: String::new(),
    }
}

// body["error"] when it is an object, otherwise the body itself
fn flattened_body(bytes: &[u8]) -> Option<RawExceptionBody> {
    let value = serde_json::from_slice::<Json>(bytes).ok()?;
    let Json::Object(mut fields) = value else {
        return None;
    };
    let flat = match fields.shift_remove("error") {
        Some(Json::Object(inner)) => inner,
        Some(_) | None => fields,
    };
    let text = |name: &str| flat.get(name).and_then(Json::as_str).map(str::to_owned);
    Some(RawExceptionBody {
        kind: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message"),
        error: None,
    })
}
