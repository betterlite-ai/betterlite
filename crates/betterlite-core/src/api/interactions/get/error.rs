use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_http::interactions::base::{ClientError, RequestFailure};
use betterlite_http::interactions::get::GetInteractionError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::agents::item::error::ProviderFailure;

#[derive(Debug, Error)]
pub enum InteractionGetError<Provider>
where
    Provider: ProviderFailure,
{
    #[error("Interactions API not supported for: {provider}")]
    Unsupported { provider: String },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Client(Box<ClientError>),
    #[error(transparent)]
    Transport(Box<RequestFailure>),
    #[error("{0}")]
    Provider(#[source] Provider),
}

impl<Provider> From<GetInteractionError<Provider>> for InteractionGetError<Provider>
where
    Provider: ProviderFailure,
{
    fn from(carried: GetInteractionError<Provider>) -> Self {
        match carried {
            GetInteractionError::Client(error) | GetInteractionError::Parameters(error) => {
                Self::Client(Box::new(error))
            }
            GetInteractionError::Provider(error) => Self::Provider(error),
            GetInteractionError::Transport(failure) => Self::Transport(Box::new(failure)),
        }
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct InteractionGetFailureError<Provider>
where
    Provider: ProviderFailure,
{
    #[source]
    pub error: InteractionGetError<Provider>,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure<Provider>(
    error: InteractionGetError<Provider>,
    provider: Option<&str>,
    redactor: &Redactor,
) -> InteractionGetFailureError<Provider>
where
    Provider: ProviderFailure,
{
    let class = match &error {
        InteractionGetError::Provider(carried) => dispatch(
            &DispatchInput::Raw(Box::new(RawException {
                status_code: Some(carried.status_code()),
                ..value_exception(carried.message(), provider)
            })),
            redactor,
        ),
        InteractionGetError::Client(carried) => match carried.as_ref() {
            ClientError::FieldType { field, expected } => {
                field_type_class(field, expected, provider)
            }
            ClientError::Tls(_) | ClientError::Build(_) | ClientError::UnknownProvider { .. } => {
                dispatch(
                    &DispatchInput::Raw(Box::new(rejected_exception(&error, provider))),
                    redactor,
                )
            }
        },
        rejected => dispatch(
            &DispatchInput::Raw(Box::new(rejected_exception(rejected, provider))),
            redactor,
        ),
    };
    InteractionGetFailureError { error, class }
}

// docs/compatibility.md rejects a value outside the declared type with a 400 naming the field
fn field_type_class(field: &str, expected: &str, provider: Option<&str>) -> ErrorClass {
    let context = ErrorContext {
        message: format!("litellm_params.{field} must be {expected}"),
        model: Some(String::new()),
        provider: provider.map(str::to_owned),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

// the ValueError get raises for an unsupported provider, and every failure outside a provider answer
fn rejected_exception<Provider>(
    error: &InteractionGetError<Provider>,
    provider: Option<&str>,
) -> RawException
where
    Provider: ProviderFailure,
{
    let mut exception = value_exception(error.to_string(), provider);
    "ValueError".clone_into(&mut exception.exception_type);
    exception
}

fn value_exception(message: String, provider: Option<&str>) -> RawException {
    RawException {
        model: String::new(),
        custom_llm_provider: provider.unwrap_or_default().to_owned(),
        error_str: message.clone(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
