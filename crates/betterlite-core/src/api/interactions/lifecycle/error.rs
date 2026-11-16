use std::convert::Infallible;
use std::error::Error as StdError;
use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_http::interactions::base::{ClientError, RequestFailure};
use betterlite_http::interactions::lifecycle::LifecycleError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::agents::item::error::ProviderFailure;

#[derive(Debug, Error)]
pub enum InteractionLifecycleError<Provider, Settlement = Infallible>
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    #[error("Interactions API not supported for: {provider}")]
    Unsupported { provider: String },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Client(Box<ClientError>),
    #[error(transparent)]
    Transport(Box<RequestFailure>),
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("{0}")]
    Provider(#[source] Provider),
    #[error("the background interaction could not be settled")]
    Settlement(#[source] Settlement),
}

impl<Provider, Settlement> From<LifecycleError<Provider>>
    for InteractionLifecycleError<Provider, Settlement>
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    fn from(carried: LifecycleError<Provider>) -> Self {
        match carried {
            LifecycleError::Client(error) | LifecycleError::Parameters(error) => {
                Self::Client(Box::new(error))
            }
            LifecycleError::Provider(error) => Self::Provider(error),
            LifecycleError::Transport(failure) => Self::Transport(Box::new(failure)),
        }
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct InteractionLifecycleFailure<Provider, Settlement = Infallible>
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    #[source]
    pub error: InteractionLifecycleError<Provider, Settlement>,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure<Provider, Settlement>(
    error: InteractionLifecycleError<Provider, Settlement>,
    provider: &str,
    redactor: &Redactor,
) -> InteractionLifecycleFailure<Provider, Settlement>
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    let class = match &error {
        InteractionLifecycleError::Client(carried) => match carried.as_ref() {
            ClientError::FieldType { field, expected } => {
                field_type_class(field, expected, provider)
            }
            ClientError::Tls(_) | ClientError::Build(_) | ClientError::UnknownProvider { .. } => {
                dispatch(
                    &DispatchInput::Raw(Box::new(value_error(&error, provider))),
                    redactor,
                )
            }
        },
        InteractionLifecycleError::Provider(carried) => dispatch(
            &DispatchInput::Raw(Box::new(RawException {
                status_code: Some(carried.status_code()),
                ..exception(carried.message(), provider)
            })),
            redactor,
        ),
        InteractionLifecycleError::Unsupported { .. }
        | InteractionLifecycleError::Logging(_)
        | InteractionLifecycleError::Transport(_)
        | InteractionLifecycleError::Timeout { .. }
        | InteractionLifecycleError::Settlement(_) => dispatch(
            &DispatchInput::Raw(Box::new(value_error(&error, provider))),
            redactor,
        ),
    };
    InteractionLifecycleFailure { error, class }
}

// docs/compatibility.md rejects a value outside the declared type with a 400 naming the field
fn field_type_class(field: &str, expected: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message: format!("litellm_params.{field} must be {expected}"),
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

// anything exception_type reads no provider status off reaches its catchall, whose status is unknown
fn value_error<Provider, Settlement>(
    error: &InteractionLifecycleError<Provider, Settlement>,
    provider: &str,
) -> RawException
where
    Provider: ProviderFailure,
    Settlement: StdError + Send + Sync + 'static,
{
    let mut raw = exception(error.to_string(), provider);
    "ValueError".clone_into(&mut raw.exception_type);
    raw
}

fn exception(message: String, provider: &str) -> RawException {
    RawException {
        // LiteLLM passes model=None, so exception_type nests under custom_llm_provider alone
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
