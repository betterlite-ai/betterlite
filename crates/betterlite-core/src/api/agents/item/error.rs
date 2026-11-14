use std::error::Error as StdError;
use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_http::interactions::agents_create_read::AgentRequestError;
use betterlite_http::interactions::agents_delete_versions::AgentsDeleteVersionsError;
use betterlite_http::interactions::base::{ClientError, RequestFailure};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::super::AgentsApiError;

// what LiteLLM's exception_type reads off a provider exception: its message and its HTTP status
pub trait ProviderFailure: StdError + Send + Sync + 'static {
    fn status_code(&self) -> u16;

    fn message(&self) -> String;
}

#[derive(Debug, Error)]
pub enum AgentItemError<Provider>
where
    Provider: ProviderFailure,
{
    #[error(transparent)]
    Unsupported(#[from] AgentsApiError),
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
    Provider(Provider),
}

impl<Provider> From<AgentRequestError<Provider>> for AgentItemError<Provider>
where
    Provider: ProviderFailure,
{
    fn from(source: AgentRequestError<Provider>) -> Self {
        match source {
            AgentRequestError::Client(error) | AgentRequestError::Parameters(error) => {
                Self::Client(Box::new(error))
            }
            AgentRequestError::Provider(error) => Self::Provider(error),
            AgentRequestError::Transport(failure) => Self::Transport(Box::new(failure)),
        }
    }
}

impl<Provider> From<AgentsDeleteVersionsError<Provider>> for AgentItemError<Provider>
where
    Provider: ProviderFailure,
{
    fn from(source: AgentsDeleteVersionsError<Provider>) -> Self {
        match source {
            AgentsDeleteVersionsError::Client(error)
            | AgentsDeleteVersionsError::Parameters(error) => Self::Client(Box::new(error)),
            AgentsDeleteVersionsError::Provider(error) => Self::Provider(error),
            AgentsDeleteVersionsError::Transport(failure) => Self::Transport(Box::new(failure)),
            AgentsDeleteVersionsError::Timeout { seconds, source } => {
                Self::Timeout { seconds, source }
            }
        }
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct AgentItemFailureError<Provider>
where
    Provider: ProviderFailure,
{
    #[source]
    pub error: AgentItemError<Provider>,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=name, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure<Provider>(
    error: AgentItemError<Provider>,
    name: &str,
    provider: Option<&str>,
    redactor: &Redactor,
) -> AgentItemFailureError<Provider>
where
    Provider: ProviderFailure,
{
    let class = match &error {
        AgentItemError::Unsupported(unsupported) => unsupported.class(),
        AgentItemError::Client(carried) => match carried.as_ref() {
            ClientError::FieldType { field, expected } => {
                field_type_class(field, expected, name, provider)
            }
            ClientError::Tls(_) | ClientError::Build(_) | ClientError::UnknownProvider { .. } => {
                dispatch(
                    &DispatchInput::Raw(Box::new(item_exception(&error, name, provider))),
                    redactor,
                )
            }
        },
        AgentItemError::Provider(carried) => dispatch(
            &DispatchInput::Raw(Box::new(provider_exception(carried, name, provider))),
            redactor,
        ),
        AgentItemError::Logging(_)
        | AgentItemError::Transport(_)
        | AgentItemError::Timeout { .. } => dispatch(
            &DispatchInput::Raw(Box::new(item_exception(&error, name, provider))),
            redactor,
        ),
    };
    AgentItemFailureError { error, class }
}

// every entry point wraps its transport's answer the same way, with the agent name and the provider
pub(crate) fn outcome<Carried, Output, Provider>(
    sent: Result<Output, Carried>,
    name: &str,
    provider: &str,
    redactor: &Redactor,
) -> Result<Output, AgentItemFailureError<Provider>>
where
    AgentItemError<Provider>: From<Carried>,
    Provider: ProviderFailure,
{
    sent.map_err(|carried| {
        failure(
            AgentItemError::from(carried),
            name,
            Some(provider),
            redactor,
        )
    })
}

// docs/compatibility.md rejects a value outside the declared type with a 400 naming the field
fn field_type_class(field: &str, expected: &str, name: &str, provider: Option<&str>) -> ErrorClass {
    let context = ErrorContext {
        message: format!("litellm_params.{field} must be {expected}"),
        model: Some(name.to_owned()),
        provider: provider.map(str::to_owned),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

fn provider_exception<Provider>(
    carried: &Provider,
    name: &str,
    provider: Option<&str>,
) -> RawException
where
    Provider: ProviderFailure,
{
    RawException {
        status_code: Some(carried.status_code()),
        ..exception(carried.message(), name, provider)
    }
}

// anything that is not a provider status reaches exception_type's catchall, whose status is unknown
fn item_exception<Provider>(
    error: &AgentItemError<Provider>,
    name: &str,
    provider: Option<&str>,
) -> RawException
where
    Provider: ProviderFailure,
{
    RawException {
        exception_type: "ValueError".to_owned(),
        ..exception(error.to_string(), name, provider)
    }
}

fn exception(message: String, name: &str, provider: Option<&str>) -> RawException {
    RawException {
        model: name.to_owned(),
        custom_llm_provider: provider.unwrap_or_default().to_owned(),
        error_str: message.clone(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
