use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::container_collection_http::ContainerCollectionError;
use crate::containers::container_request_parameters::ContainerFieldTypeError;

// exception_type() maps every create and list failure with model "" and the caller's custom_llm_provider
const MODEL: &str = "";

// LiteLLM's mock parse, then ContainerObject(**mock_response) or ContainerListResponse(**mock_response)
#[derive(Debug, Error)]
pub enum MockDecodeError {
    #[error("the mock response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match {expected}")]
    Model {
        expected: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

// what create_container, acreate_container, list_containers and alist_containers raise before exception_type sees it
#[derive(Debug, Error)]
pub enum ContainerOpError {
    #[error(transparent)]
    Mock(#[from] MockDecodeError),
    // LlmProviders(custom_llm_provider) rejects a name outside the provider set
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    // the create body's ValueError
    #[error("container operations are not supported for {provider}")]
    Unsupported { provider: String },
    // the list body's ValueError, capital C as LiteLLM writes it
    #[error("Container provider config not found for provider: {provider}")]
    ConfigMissing { provider: String },
    // B3361's encode_container_id_in_response refused a value the declared type cannot hold
    #[error(transparent)]
    Encode(#[from] ContainerFieldTypeError),
    // the Json, JsonValue and Object bridges in prepare.rs; a failure here is local, not a provider answer
    #[error(transparent)]
    Params(#[from] serde_json::Error),
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(#[from] ContainerCollectionError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct ContainerOpFailure {
    #[source]
    pub error: ContainerOpError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...), run once
#[must_use]
pub fn failure(error: ContainerOpError, provider: &str, redactor: &Redactor) -> ContainerOpFailure {
    let class = match &error {
        ContainerOpError::Encode(field_error) => {
            named_field_class(error.to_string(), field_error.field, provider)
        }
        ContainerOpError::Handler(ContainerCollectionError::Params(
            ContainerFileHttpError::ParamType { field, .. },
        )) => named_field_class(error.to_string(), field, provider),
        ContainerOpError::Handler(handler_error) => {
            handler_class(handler_error, provider, redactor)
        }
        ContainerOpError::Mock(_)
        | ContainerOpError::UnknownProvider { .. }
        | ContainerOpError::Unsupported { .. }
        | ContainerOpError::ConfigMissing { .. }
        | ContainerOpError::Params(_)
        | ContainerOpError::Timeout { .. }
        | ContainerOpError::Logging(_) => value_error(error.to_string(), provider, redactor),
    };
    ContainerOpFailure { error, class }
}

// a provider or config upstream exception keeps its status, and every other handler failure is the catchall
fn handler_class(
    error: &ContainerCollectionError,
    provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    let upstream = match error {
        ContainerCollectionError::Provider(provider_error) => provider_error,
        ContainerCollectionError::Config(ContainerError::Upstream(provider_error)) => {
            provider_error
        }
        ContainerCollectionError::Config(_)
        | ContainerCollectionError::Params(_)
        | ContainerCollectionError::Client(_)
        | ContainerCollectionError::Logging(_)
        | ContainerCollectionError::Encode(_)
        | ContainerCollectionError::Suspended => {
            return value_error(error.to_string(), provider, redactor);
        }
    };
    dispatch(
        &DispatchInput::Raw(Box::new(exception(
            upstream.message.clone(),
            provider,
            Some(upstream.status_code),
        ))),
        redactor,
    )
}

fn value_error(message: String, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception(message, provider, None);
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

fn exception(message: String, provider: &str, status_code: Option<u16>) -> RawException {
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
