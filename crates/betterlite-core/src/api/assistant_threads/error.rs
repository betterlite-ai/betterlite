use std::collections::BTreeMap;

use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::TimeoutError;

const UNSUPPORTED_MODEL: &str = "n/a";

#[derive(Debug, Error)]
pub enum ThreadError {
    #[error("LiteLLM doesn't support {provider} for '{operation}'. Only 'openai' is supported.")]
    Unsupported {
        provider: String,
        operation: &'static str,
    },
    #[error("{message}")]
    Params {
        field: Option<String>,
        message: String,
    },
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("{message}")]
    Provider {
        message: String,
        failure: ProviderFailure,
    },
    #[error("the thread call failed: {message}")]
    Call { message: String },
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
    let body_text = String::from_utf8_lossy(body).into_owned();
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
    let fields = match object.remove("error") {
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
            Self::Unsupported { .. } | Self::Params { .. } | Self::Timeout(_) => SdkFailure {
                class: "BadRequestError",
                status: 400,
                kind: None,
                param: None,
                code: None,
                message: self.to_string(),
            },
            // a broken transport or reply decodes as an APIError, but no provider answered with a status
            Self::Call { message } => SdkFailure {
                class: "APIError",
                status: 500,
                kind: None,
                param: None,
                code: None,
                message: message.clone(),
            },
            Self::Reply => SdkFailure {
                class: "APIError",
                status: 500,
                kind: None,
                param: None,
                code: None,
                message: self.to_string(),
            },
        }
    }

    // acreate_thread and aget_thread run exception_type() over whatever the synchronous facade raised
    #[must_use]
    pub fn error_class(&self, provider: &str, redactor: &Redactor) -> ErrorClass {
        match self {
            Self::Unsupported { .. } => ErrorClass::BadRequest(Box::new(
                ResponseDetails::with_status(self.context(provider), Some(400)),
            )),
            Self::Params { field, message } => {
                ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
                    ErrorContext {
                        message: message.clone(),
                        model: None,
                        provider: Some(provider.to_owned()),
                        max_retries: None,
                        num_retries: None,
                        body_type: None,
                        body_param: field.clone(),
                        body_code: None,
                    },
                    Some(400),
                )))
            }
            Self::Timeout(_) => ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
                ErrorContext {
                    message: self.to_string(),
                    model: None,
                    provider: Some(provider.to_owned()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                },
                Some(400),
            ))),
            Self::Provider {
                message, failure, ..
            } => dispatch(
                &DispatchInput::Raw(Box::new(failure.raw(provider, message))),
                redactor,
            ),
            // no provider answered, so exception_type() has no status to branch on; keep it an internal failure
            Self::Call { .. } | Self::Reply => ErrorClass::InternalServer(Box::new(ErrorContext {
                message: self.to_string(),
                model: None,
                provider: Some(provider.to_owned()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            })),
        }
    }

    // Litellm raises the unsupported-provider error with model "n/a" and an empty response body
    fn context(&self, provider: &str) -> ErrorContext {
        ErrorContext {
            message: self.to_string(),
            model: Some(UNSUPPORTED_MODEL.to_owned()),
            provider: Some(provider.to_owned()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        }
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
