use std::time::TryFromFloatSecsError;

use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_core::api::eval_run_create_list::error::{EvalRunCallError, EvalRunFailure};
use betterlite_core::api::eval_run_lifecycle::error::{
    EvalRunError, EvalRunFailure as RunFailure, failure as lifecycle_failure,
};
use betterlite_error::envelope::WireField;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::metadata::ErrorMetadata;
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_logging::logging_state::LoggingStateError;
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

create_exception!(betterlite_native, NativeEvalRunError, PyException);

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
    #[error("the eval run answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        if let SessionError::Json(inner) = &error {
            return PyValueError::new_err(inner.to_string());
        }
        PyRuntimeError::new_err(error.to_string())
    }
}

// Everything one run operation can fail with: its core service's class, or a session error before or after the call.
#[derive(Debug)]
pub(crate) enum EvalOperationFailure {
    Mapped { class: ErrorClass, provider: String },
    Session(SessionError),
}

impl From<SessionError> for EvalOperationFailure {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl From<ConvertError> for EvalOperationFailure {
    fn from(error: ConvertError) -> Self {
        let message = error.to_string();
        match error {
            ConvertError::Json { source } => {
                Self::Session(SessionError::Json(ConvertError::Json { source }))
            }
            ConvertError::FieldType { provider, name } => Self::Mapped {
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

// Which core service a call's logging state belongs to, so its failure is mapped the way that service maps it.
#[derive(Clone, Copy)]
pub(crate) enum FailureGroup {
    CreateList,
    Lifecycle,
}

impl FailureGroup {
    pub(crate) fn logging(
        self,
        error: LoggingStateError,
        provider: &str,
        redactor: &Redactor,
    ) -> ErrorClass {
        match self {
            Self::CreateList => {
                EvalRunFailure::of(
                    EvalRunCallError::Logging(error),
                    provider,
                    redactor,
                    IndexMap::new(),
                )
                .class
            }
            Self::Lifecycle => {
                lifecycle_failure(EvalRunError::Logging(error), provider, redactor).class
            }
        }
    }
}

// The failure class a core run service hands back for one reply.
pub(crate) trait Classified {
    fn into_class(self) -> ErrorClass;
}

impl Classified for EvalRunFailure {
    fn into_class(self) -> ErrorClass {
        self.class
    }
}

impl Classified for RunFailure {
    fn into_class(self) -> ErrorClass {
        self.class
    }
}

impl EvalOperationFailure {
    // The core service's answer as the JSON text Python validates, or the class its failure names.
    pub(crate) fn reply<T, E>(answer: Result<T, E>, provider: &str) -> Result<String, Self>
    where
        T: Serialize,
        E: Classified,
    {
        match answer {
            Ok(value) => serde_json::to_string(&value)
                .map_err(|source| Self::Session(SessionError::Encode { source })),
            Err(failure) => Err(Self::Mapped {
                class: failure.into_class(),
                provider: provider.to_owned(),
            }),
        }
    }
}

impl From<EvalOperationFailure> for PyErr {
    fn from(failure: EvalOperationFailure) -> Self {
        match failure {
            EvalOperationFailure::Mapped { class, provider } => native_error(&class, &provider),
            EvalOperationFailure::Session(error) => Self::from(error),
        }
    }
}

// What _completion/errors.py rebuilds into a LiteLLM exception: its class, message, status and error body.
#[derive(Debug, Serialize)]
struct RunErrorPayload<'text> {
    #[serde(rename = "type")]
    class: &'text str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_code: Option<u16>,
    llm_provider: &'text str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<Json>,
}

// The provider error class Python raises, keeping the class and cause exception_type chose for it.
pub(crate) fn native_error(class: &ErrorClass, provider: &str) -> PyErr {
    let metadata = class.metadata();
    let body = class_body(&metadata);
    let payload = RunErrorPayload {
        class: class_name(class),
        message: class_message(class),
        status_code: class_status(class),
        llm_provider: provider,
        // exception_type is called with model=None, so the empty model a dispatch carries is not sent
        model: metadata.model.filter(|name| !name.is_empty()),
        body,
    };
    match serde_json::to_string(&payload) {
        Ok(text) => NativeEvalRunError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(class_message(class)),
    }
}

// The flat error object openai.APIError reads type, param and code from; absent fields stay absent.
fn class_body(metadata: &ErrorMetadata) -> Option<Json> {
    let mut body = IndexMap::new();
    for (name, field) in [("type", &metadata.error_type), ("param", &metadata.param)] {
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
    if let Some(code) = metadata.openai_code.as_deref() {
        body.insert("code".to_owned(), Json::Text(code.to_owned()));
    }
    (!body.is_empty()).then_some(Json::Object(body))
}
