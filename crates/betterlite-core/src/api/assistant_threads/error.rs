use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::details::{ResponseDetails, TimeoutDetails, TimeoutStatus};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::TimeoutError;

use super::params::ThreadParamsError;

const UNSUPPORTED_MODEL: &str = "n/a";

#[derive(Debug, Error)]
pub enum ThreadError {
    #[error("LiteLLM doesn't support {provider} for '{operation}'. Only 'openai' is supported.")]
    Unsupported {
        provider: String,
        operation: &'static str,
    },
    #[error(transparent)]
    Params(#[from] ThreadParamsError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("{message}")]
    Provider {
        message: String,
        failure: Box<ProviderFailure>,
    },
    // the call failed without a provider status; exception_type() classifies it from the source
    #[error("the thread call failed: {0}")]
    Threads(#[from] ThreadsError),
    // a deadline or connect timeout is the SDK's APITimeoutError, which exception_type() raises as Timeout
    #[error("the thread call failed: {0}")]
    ProviderTimeout(#[source] ThreadsError),
    #[error("the thread call failed: {0}")]
    Definitions(#[from] DefinitionsError),
    #[error("the provider returned no thread object")]
    Reply,
}

// what acreate_thread and aget_thread raise: the source error next to the class exception_type() produced
#[derive(Debug)]
pub struct ThreadFailure {
    pub error: ThreadError,
    pub class: ErrorClass,
}

// The OpenAI SDK's APIStatusError as the synchronous facade raises it: a class by status and the flat body's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    pub status: u16,
    pub body_text: String,
    pub kind: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
    pub body_message: Option<String>,
}

#[must_use]
pub fn provider_failure(status: u16, body: &[u8]) -> ProviderFailure {
    let body_text = from_utf8(body).map_or_else(|_| String::new(), String::from);
    let flat = flat_body(&body_text);
    let field = |name: &str| {
        flat.as_ref()
            .and_then(|object| object.get(name))
            .and_then(text_of)
            .map(str::to_owned)
    };
    ProviderFailure {
        status,
        kind: field("type"),
        param: field("param"),
        code: field("code"),
        body_message: field("message"),
        body_text,
    }
}

fn text_of(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// the SDK's _make_status_error takes body["error"] whenever the key exists; APIError reads fields only off a mapping
fn flat_body(text: &str) -> Option<BTreeMap<String, Json>> {
    let Json::Object(mut object) = serde_json::from_str::<Json>(text).ok()? else {
        return None;
    };
    let fields = match object.shift_remove("error") {
        Some(Json::Object(fields)) => fields,
        // the key is there but its value is not a mapping, so the SDK's APIError gets no fields at all
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return None;
        }
        None => object,
    };
    Some(fields.into_iter().collect())
}

// the openai.APIStatusError subclass the pinned SDK builds for a status, before LiteLLM maps anything
#[must_use]
pub const fn sdk_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500.. => "InternalServerError",
        _ => "APIStatusError",
    }
}

// The synchronous facade raises the SDK's exception as it is, so the oracle and the routes read these fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkFailure {
    pub class: &'static str,
    pub status: u16,
    pub kind: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

impl ThreadError {
    #[must_use]
    pub fn sdk_failure(&self) -> SdkFailure {
        let raised = self.to_string();
        match self {
            Self::Provider {
                message, failure, ..
            } => SdkFailure {
                class: sdk_class(failure.status),
                status: failure.status,
                kind: failure.kind.clone(),
                param: failure.param.clone(),
                code: failure.code.clone(),
                message: message.clone(),
            },
            // docs/compatibility.md names the field with a 400 where LiteLLM's pydantic model raises
            Self::Unsupported { .. } | Self::Params(_) | Self::Timeout(_) => {
                status_failure("BadRequestError", 400, &raised)
            }
            // exception_type() raises an AuthenticationError for the missing api key
            Self::Threads(ThreadsError::MissingApiKey) => {
                status_failure("AuthenticationError", 401, &raised)
            }
            Self::ProviderTimeout(_) => status_failure("APITimeoutError", 408, &raised),
            // every other call failure carries no status, which exception_type() reads as APIConnectionError
            Self::Threads(_) | Self::Definitions(_) => {
                status_failure("APIConnectionError", 500, &raised)
            }
            Self::Reply => status_failure("APIError", 500, &raised),
        }
    }

    // acreate_thread and aget_thread run exception_type() over whatever the synchronous facade raised
    #[must_use]
    pub fn error_class(&self, provider: &str, redactor: &Redactor) -> ErrorClass {
        let bad_request = |model: Option<String>, body_param: Option<String>| {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
                self.context(provider, model, body_param),
                Some(400),
            )))
        };
        match self {
            Self::Unsupported { .. } => bad_request(Some(UNSUPPORTED_MODEL.to_owned()), None),
            Self::Params(error) => bad_request(None, error.field.clone()),
            Self::Timeout(_) => bad_request(None, None),
            Self::Provider {
                message, failure, ..
            } => dispatch(
                &DispatchInput::Raw(Box::new(failure.raw(provider, message))),
                redactor,
            ),
            // exception_type() raises an AuthenticationError for the missing api key
            Self::Threads(ThreadsError::MissingApiKey) => ErrorClass::Authentication(Box::new(
                ResponseDetails::with_status(self.context(provider, None, None), Some(401)),
            )),
            // a deadline or connect timeout is the SDK's APITimeoutError, which maps as Timeout
            Self::ProviderTimeout(_) => ErrorClass::Timeout(Box::new(TimeoutDetails {
                context: self.context(provider, None, None),
                original_status: TimeoutStatus(408),
            })),
            // no status reached exception_type(), which falls back to APIConnectionError
            Self::Threads(_) | Self::Definitions(_) => {
                ErrorClass::ApiConnection(Box::new(self.context(provider, None, None)))
            }
            Self::Reply => ErrorClass::InternalServer(Box::new(self.context(provider, None, None))),
        }
    }

    // Litellm raises the unsupported-provider error with model "n/a" and an empty response body
    fn context(
        &self,
        provider: &str,
        model: Option<String>,
        body_param: Option<String>,
    ) -> ErrorContext {
        ErrorContext {
            message: self.to_string(),
            model,
            provider: Some(provider.to_owned()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param,
            body_code: None,
        }
    }
}

// the SDK exception no provider status produced: a class, a fixed status and the raised message
fn status_failure(class: &'static str, status: u16, message: &str) -> SdkFailure {
    SdkFailure {
        class,
        status,
        kind: None,
        param: None,
        code: None,
        message: message.to_owned(),
    }
}

impl ProviderFailure {
    // exception_type()'s kwargs for the pinned SDK exception: model "", the raised message and the flat body
    fn raw(&self, provider: &str, message: &str) -> RawException {
        RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: message.to_owned(),
            status_code: Some(self.status),
            status_code_is_synthesized: false,
            message: Some(message.to_owned()),
            body: Some(RawExceptionBody {
                kind: self.kind.clone(),
                param: self.param.clone(),
                code: self.code.clone(),
                message: self.body_message.clone(),
                error: None,
            }),
            exception_type: sdk_class(self.status).to_owned(),
        }
    }
}
