use std::error::Error;
use std::fmt;

use betterlite_core::api::assistant_messages::error::MessageFailure;
use betterlite_core::api::assistant_read_delete::error::AssistantReadDeleteError;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::AssistantSdkError;
use betterlite_core::api::assistant_records::error::AssistantCreateFailure;
use betterlite_core::api::assistant_run_sdk::error::AssistantRunError;
use betterlite_core::api::assistant_threads::error::ThreadFailure;
use betterlite_error::envelope::WireField;
use betterlite_openai::assistants::error::AssistantsError;
use serde::Deserialize;

// The exception the SDK owner raised, with the attributes the legacy proxy handler reads off it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantFailure {
    pub class: String,
    pub status: Option<u16>,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

impl fmt::Display for AssistantFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<AssistantCreateFailure> for AssistantFailure {
    fn from(failure: AssistantCreateFailure) -> Self {
        Self {
            class: failure.class,
            status: failure.status,
            kind: failure.kind,
            param: failure.param,
            code: failure.code,
            message: failure.message,
        }
    }
}

#[derive(Debug)]
pub enum AssistantDispatchError {
    // the factory's own Exception: it has no status_code, type, param or code attribute
    Exception { message: String },
    // assistants_config["custom_llm_provider"] and ["litellm_params"] are plain dictionary reads
    KeyError { key: &'static str },
    // duplicate keywords and missing required arguments are the interpreter's own errors
    TypeError { message: String },
    // the typed SDK input refused a value; the message names the field
    Invalid { message: String },
    Failure(AssistantFailure),
    ReadDelete(AssistantReadDeleteError<AssistantSdkError>),
    Thread(Box<ThreadFailure>),
    Message(MessageFailure),
    Run(AssistantRunError),
}

impl AssistantDispatchError {
    #[must_use]
    pub const fn key_error(key: &'static str) -> Self {
        Self::KeyError { key }
    }

    // the wrapper call itself: an explicit path argument collides with the same key in the body
    #[must_use]
    pub fn multiple_values(name: &str) -> Self {
        Self::TypeError {
            message: format!(
                "litellm.router.Router.factory_function.<locals>.async_wrapper() got multiple values for keyword argument '{name}'"
            ),
        }
    }

    // the merged call into the installed function: its own named arguments collide with config parameters
    #[must_use]
    pub fn duplicate_argument(function: &str, name: &str) -> Self {
        Self::TypeError {
            message: format!("{function}() got multiple values for keyword argument '{name}'"),
        }
    }

    #[must_use]
    pub fn missing_argument(function: &str, name: &str) -> Self {
        Self::TypeError {
            message: format!("{function}() missing 1 required positional argument: '{name}'"),
        }
    }

    #[must_use]
    pub fn invalid(field: &str) -> Self {
        Self::Invalid {
            message: format!("'{field}' has the wrong type"),
        }
    }

    #[must_use]
    pub fn invalid_message(message: String) -> Self {
        Self::Invalid { message }
    }

    // get_assistants and delete_assistant hand their status failure on as the async wrapper mapped it
    #[must_use]
    pub fn read_delete(error: AssistantReadDeleteError<AssistantSdkError>) -> Self {
        if let Some(fields) = error.fields() {
            let text = error.to_string();
            let message = if fields.prefixed {
                format!("litellm.{}: {text}", fields.class)
            } else {
                text
            };
            return Self::Failure(AssistantFailure {
                class: fields.class.to_owned(),
                status: fields.status,
                kind: fields
                    .error_type
                    .map_or(WireField::Missing, |kind| WireField::Text(kind.to_owned())),
                param: WireField::Missing,
                code: WireField::Missing,
                message,
            });
        }
        match &error {
            AssistantReadDeleteError::Dispatch(AssistantSdkError::Assistants(
                AssistantsError::Status { status, body },
            )) => Self::Failure(status_failure(*status, body)),
            AssistantReadDeleteError::Limits(_)
            | AssistantReadDeleteError::Timeout(_)
            | AssistantReadDeleteError::Unsupported { .. }
            | AssistantReadDeleteError::InvalidFlag { .. }
            | AssistantReadDeleteError::MissingAssistantId
            | AssistantReadDeleteError::MissingReply { .. }
            | AssistantReadDeleteError::MismatchedReply
            | AssistantReadDeleteError::Dispatch(_) => Self::ReadDelete(error),
        }
    }
}

// the async wrapper's exception_type mapping leaves type, param and code null for a status failure
fn status_failure(status: u16, body: &[u8]) -> AssistantFailure {
    AssistantFailure {
        class: status_class(status).to_owned(),
        status: Some(status),
        kind: WireField::Null,
        param: WireField::Null,
        code: WireField::Null,
        message: body_message(body),
    }
}

const fn status_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=599 => "InternalServerError",
        _ => "APIStatusError",
    }
}

fn body_message(body: &[u8]) -> String {
    let parsed = serde_json::from_slice::<ErrorBody>(body).ok();
    let message = parsed
        .and_then(|body| body.error)
        .and_then(|error| error.message);
    message.unwrap_or_else(|| str::from_utf8(body).unwrap_or_default().to_owned())
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorDetail>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: Option<String>,
}

impl fmt::Display for AssistantDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exception { message }
            | Self::TypeError { message }
            | Self::Invalid { message } => formatter.write_str(message),
            Self::KeyError { key } => write!(formatter, "'{key}'"),
            Self::Failure(failure) => fmt::Display::fmt(failure, formatter),
            Self::ReadDelete(error) => fmt::Display::fmt(error, formatter),
            Self::Thread(failure) => fmt::Display::fmt(&failure.error, formatter),
            Self::Message(failure) => fmt::Display::fmt(failure, formatter),
            Self::Run(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl Error for AssistantDispatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ReadDelete(error) => Some(error),
            Self::Message(failure) => Some(failure),
            Self::Run(error) => Some(error),
            Self::Exception { .. }
            | Self::KeyError { .. }
            | Self::TypeError { .. }
            | Self::Invalid { .. }
            | Self::Failure(_)
            | Self::Thread(_) => None,
        }
    }
}
