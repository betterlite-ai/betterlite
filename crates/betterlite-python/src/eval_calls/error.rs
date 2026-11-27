use std::time::TryFromFloatSecsError;

use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_core::api::eval_create_list::DEFAULT_EVALS_PROVIDER;
use betterlite_core::api::eval_create_list::call::EvalsCallParams;
use betterlite_core::api::eval_create_list::error::{
    EvalsCreateListError, EvalsCreateListFailureError, failure as create_list_failure,
};
use betterlite_core::api::eval_get_update::error::{
    EvalsGetUpdateError, EvalsGetUpdateFailureError, failure as get_update_failure,
};
use betterlite_core::api::eval_lifecycle::error::{
    EvalLifecycleError, EvalLifecycleFailure, failure as lifecycle_failure,
};
use betterlite_error::envelope::WireField;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use serde::Serialize;

use super::convert::ConvertError;
use crate::completion::error::SessionError as CompletionSessionError;

create_exception!(betterlite_native, NativeEvalError, PyException);

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Client(#[from] CompletionSessionError),
    #[error("the request timeout could not be resolved: {0}")]
    Timeout(#[from] TimeoutError),
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Duration {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the redactor could not be built: {0}")]
    Redaction(#[from] RedactionError),
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        match &error {
            SessionError::Json(_) => PyValueError::new_err(error.to_string()),
            SessionError::Client(_)
            | SessionError::Timeout(_)
            | SessionError::Duration { .. }
            | SessionError::Redaction(_) => PyRuntimeError::new_err(error.to_string()),
        }
    }
}

// Everything one eval operation can fail with: a wrong-typed argument, its payload, a core service's failure, or an answer that would not encode.
#[derive(Debug)]
pub(crate) enum EvalOperationFailure {
    Request {
        class: ErrorClass,
        provider: String,
    },
    Session(#[from] SessionError),
    CreateList {
        failure: EvalsCreateListFailureError,
        provider: String,
    },
    GetUpdate {
        failure: EvalsGetUpdateFailureError,
        provider: String,
    },
    Lifecycle {
        failure: EvalLifecycleFailure,
        provider: String,
    },
    Encode {
        source: serde_json::Error,
    },
}

impl From<ConvertError> for EvalOperationFailure {
    fn from(error: ConvertError) -> Self {
        let message = error.to_string();
        match error {
            ConvertError::Json { source } => {
                Self::Session(SessionError::Json(ConvertError::Json { source }))
            }
            ConvertError::FieldType { provider, name } => Self::Request {
                class: field_class(message, name, &provider),
                provider,
            },
        }
    }
}

// docs/compatibility.md reads a request value outside its declared type as a 400 that names the field
fn field_class(message: String, name: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message,
        model: None,
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(name.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

impl EvalOperationFailure {
    // A failure before the service ran: the logging state could not be built, which LiteLLM's exception_type maps the same way.
    pub(crate) fn create_list(
        error: EvalsCreateListError,
        params: &EvalsCallParams<'_>,
        redactor: &Redactor,
    ) -> Self {
        Self::CreateList {
            failure: create_list_failure(error, params, redactor),
            provider: provider_of(params),
        }
    }

    pub(crate) fn get_update(
        error: EvalsGetUpdateError,
        params: &EvalsCallParams<'_>,
        redactor: &Redactor,
    ) -> Self {
        Self::GetUpdate {
            failure: get_update_failure(error, params, redactor),
            provider: provider_of(params),
        }
    }

    pub(crate) fn lifecycle(
        error: EvalLifecycleError,
        params: &EvalsCallParams<'_>,
        redactor: &Redactor,
    ) -> Self {
        let provider = provider_of(params);
        Self::Lifecycle {
            failure: lifecycle_failure(error, &provider, redactor),
            provider,
        }
    }

    pub(crate) fn create_list_reply<T: Serialize>(
        reply: Result<T, EvalsCreateListFailureError>,
        params: &EvalsCallParams<'_>,
    ) -> Result<String, Self> {
        reply
            .map_err(|failure| Self::CreateList {
                failure,
                provider: provider_of(params),
            })
            .and_then(encode)
    }

    pub(crate) fn get_update_reply<T: Serialize>(
        reply: Result<T, EvalsGetUpdateFailureError>,
        params: &EvalsCallParams<'_>,
    ) -> Result<String, Self> {
        reply
            .map_err(|failure| Self::GetUpdate {
                failure,
                provider: provider_of(params),
            })
            .and_then(encode)
    }

    pub(crate) fn lifecycle_reply<T: Serialize>(
        reply: Result<T, EvalLifecycleFailure>,
        params: &EvalsCallParams<'_>,
    ) -> Result<String, Self> {
        reply
            .map_err(|failure| Self::Lifecycle {
                failure,
                provider: provider_of(params),
            })
            .and_then(encode)
    }

    pub(crate) fn into_py(self) -> PyErr {
        match self {
            Self::Request { class, provider } => native_error(&class, &provider),
            Self::Session(error) => error.into(),
            Self::CreateList { failure, provider } => native_error(&failure.class, &provider),
            Self::GetUpdate { failure, provider } => native_error(&failure.class, &provider),
            Self::Lifecycle { failure, provider } => native_error(&failure.class, &provider),
            Self::Encode { source } => PyRuntimeError::new_err(source.to_string()),
        }
    }
}

fn provider_of(params: &EvalsCallParams<'_>) -> String {
    params
        .custom_llm_provider
        .unwrap_or(DEFAULT_EVALS_PROVIDER)
        .to_owned()
}

fn encode<T: Serialize>(value: T) -> Result<String, EvalOperationFailure> {
    serde_json::to_string(&value).map_err(|source| EvalOperationFailure::Encode { source })
}

// What _completion/errors.py rebuilds into a LiteLLM exception: its class, message, status and error body.
#[derive(Debug, Serialize)]
pub(crate) struct NativeErrorPayload {
    #[serde(rename = "type")]
    class: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_code: Option<u16>,
    llm_provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<Json>,
}

// The provider error class Python raises, keeping the class and cause exception_type chose for it.
pub(crate) fn native_error(class: &ErrorClass, provider: &str) -> PyErr {
    let payload = payload(class, provider);
    match serde_json::to_string(&payload) {
        Ok(text) => NativeEvalError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(class_message(class)),
    }
}

pub(crate) fn payload(class: &ErrorClass, provider: &str) -> NativeErrorPayload {
    let metadata = class.metadata();
    NativeErrorPayload {
        class: class_name(class).to_owned(),
        message: class_message(class),
        status_code: class_status(class),
        llm_provider: provider.to_owned(),
        // exception_type is called with model=None, so the empty model a dispatch carries is not sent
        model: metadata.model.filter(|name| !name.is_empty()),
        body: error_body(class),
    }
}

// The flat error object openai.APIError reads type, param and code from; absent fields stay absent.
fn error_body(class: &ErrorClass) -> Option<Json> {
    let metadata = class.metadata();
    let code = metadata
        .openai_code
        .as_deref()
        .map_or(WireField::MISSING, |text| WireField::Text(text.to_owned()));
    let mut body = IndexMap::new();
    insert_field(&mut body, "type", &metadata.error_type);
    insert_field(&mut body, "param", &metadata.param);
    insert_field(&mut body, "code", &code);
    (!body.is_empty()).then_some(Json::Object(body))
}

fn insert_field(body: &mut IndexMap<String, Json>, name: &str, field: &WireField) {
    match field {
        WireField::Missing(_) => {}
        WireField::Null => {
            body.insert(name.to_owned(), Json::Null);
        }
        WireField::Text(text) => {
            body.insert(name.to_owned(), Json::Text(text.clone()));
        }
    }
}
