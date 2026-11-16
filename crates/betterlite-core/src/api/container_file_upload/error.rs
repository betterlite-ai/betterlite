use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::execution::UploadFailure;
use crate::containers::container_request_parameters::ContainerFieldTypeError;

// exception_type() maps every upload failure with model "" and the provider the call resolved to
const MODEL: &str = "";

// LiteLLM's mock parse, then ContainerFileObject(**mock_response), whose failures are ValueErrors too
#[derive(Debug, Error)]
pub enum MockDecodeError {
    #[error("the mock response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match a container file")]
    Model {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Error)]
pub enum ContainerFileUploadError<Failure>
where
    Failure: UploadFailure,
{
    #[error(transparent)]
    Mock(#[from] MockDecodeError),
    #[error(transparent)]
    Decode(#[from] ContainerFieldTypeError),
    #[error("Container provider config not found for provider: {provider}")]
    ProviderConfigMissing { provider: String },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error("{0}")]
    Provider(#[source] Failure),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct ContainerFileUploadFailure<Failure>
where
    Failure: UploadFailure,
{
    #[source]
    pub error: ContainerFileUploadError<Failure>,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure<Failure>(
    error: ContainerFileUploadError<Failure>,
    provider: &str,
    redactor: &Redactor,
) -> ContainerFileUploadFailure<Failure>
where
    Failure: UploadFailure,
{
    let class = match &error {
        ContainerFileUploadError::Decode(field_error) => field_type_class(field_error, provider),
        ContainerFileUploadError::Provider(carried) => dispatch(
            &DispatchInput::Raw(Box::new(provider_exception(carried, provider))),
            redactor,
        ),
        ContainerFileUploadError::Mock(_)
        | ContainerFileUploadError::ProviderConfigMissing { .. }
        | ContainerFileUploadError::Logging(_) => dispatch(
            &DispatchInput::Raw(Box::new(value_exception(&error, provider))),
            redactor,
        ),
    };
    ContainerFileUploadFailure { error, class }
}

// docs/compatibility.md rejects a value outside its declared type with a 400 that names the field
fn field_type_class(error: &ContainerFieldTypeError, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message: error.to_string(),
        model: Some(MODEL.to_owned()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(error.field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

fn provider_exception<Failure>(carried: &Failure, provider: &str) -> RawException
where
    Failure: UploadFailure,
{
    let message = carried.message();
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code: carried.status_code(),
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}

// the ValueError LiteLLM raises itself, or the ValueError subclass pydantic raises: the status is unknown
fn value_exception<Failure>(
    error: &ContainerFileUploadError<Failure>,
    provider: &str,
) -> RawException
where
    Failure: UploadFailure,
{
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: error.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(error.to_string()),
        body: None,
        exception_type: String::from("ValueError"),
    }
}
