use super::convert::{ConvertError, NativeEmbeddingErrorPayload};
use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_core::operation::embedding::route_error;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::{ResponseDetails, TimeoutDetails, TimeoutStatus};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::embedding::EmbeddingError;
use betterlite_provider::operation::OperationError;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
create_exception!(betterlite_native, NativeEmbeddingError, PyException);
// openai.OpenAI's own message when every key source is empty; with no status it maps to APIConnectionError
const MISSING_CREDENTIALS: &str = "Missing credentials. Please pass an `api_key`, `workload_identity`, `admin_api_key`, or set the `OPENAI_API_KEY` or `OPENAI_ADMIN_KEY` environment variable.";
const TIMEOUT_ERROR: &str = "APITimeoutError - Request timed out. Error_str: Request timed out.";
// the identity every answer and every error carries back to Python
pub(crate) struct CallFacts<'facts> {
    pub(crate) call_id: &'facts str,
    pub(crate) model: &'facts str,
    pub(crate) provider: &'facts str,
}
// One failed call: a LiteLLM exception class, or a value main.py must reject before the provider runs.
#[derive(Debug)]
pub(crate) enum CallFailure {
    Class(Box<ErrorClass>),
    Invalid(String),
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error(transparent)]
    Completion(#[from] CompletionSessionError),
    #[error("the redactor could not be built: {source}")]
    Redact {
        #[from]
        source: RedactionError,
    },
}
// The LiteLLM exception classes main.py raises itself, around the provider's own errors.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Raised {
    BadRequest,
    InternalServer,
    Timeout,
    UnsupportedParams,
    Plain,
}
pub(crate) fn raised(
    kind: Raised,
    message: String,
    model: Option<String>,
    provider: Option<String>,
) -> CallFailure {
    let context = ErrorContext {
        message,
        model,
        provider,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    let class = match kind {
        Raised::BadRequest => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::<400>::from(context)))
        }
        Raised::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        // main.py's exception_type renders a request timeout as LiteLLM's 408 Timeout
        Raised::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408_u16),
        })),
        Raised::UnsupportedParams => ErrorClass::UnsupportedParams(Box::new(context)),
        // LiteLLM's mock "error" answer is a plain Exception, which carries neither a status nor a body
        Raised::Plain => ErrorClass::Unclassified(Box::new(context)),
    };
    CallFailure::Class(Box::new(class))
}
pub(crate) fn call_failure(failure: &CallFailure) -> PyErr {
    match failure {
        CallFailure::Class(class) => class_error(class),
        CallFailure::Invalid(message) => PyValueError::new_err(message.clone()),
    }
}
fn class_error(class: &ErrorClass) -> PyErr {
    let payload = NativeEmbeddingErrorPayload::from_class(class);
    match serde_json::to_string(&payload) {
        Ok(text) => NativeEmbeddingError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}
impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        let message = error.to_string();
        match error {
            SessionError::Json(ConvertError::Json { .. }) => PyValueError::new_err(message),
            SessionError::Json(ConvertError::Encode { .. })
            | SessionError::Completion(_)
            | SessionError::Redact { .. } => PyRuntimeError::new_err(message),
        }
    }
}
pub(crate) fn resolution_failure(error: &ResolutionError, model: &str) -> CallFailure {
    raised(
        Raised::BadRequest,
        error.to_string(),
        Some(model.to_owned()),
        None,
    )
}
// Every prepare, execute and decode failure, as the LiteLLM exception class main.py re-raises.
pub(crate) fn operation_failure(
    error: OperationError,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> CallFailure {
    match error {
        OperationError::MissingApiKey => {
            classified(String::from(MISSING_CREDENTIALS), None, facts, redactor)
        }
        // the provider's own error body, rendered the way the embeddings route renders it
        OperationError::Provider { response } => {
            let route = route_error(&EmbeddingError::Provider { response }, redactor);
            classified(route.message, Some(route.failure.status), facts, redactor)
        }
        OperationError::Timeout => raised(
            Raised::Timeout,
            String::from(TIMEOUT_ERROR),
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        ),
        OperationError::Http { source } => classified(source.to_string(), None, facts, redactor),
        kept @ (OperationError::UnavailableAdapter { .. }
        | OperationError::UnsupportedOperation { .. }
        | OperationError::Encode { .. }
        | OperationError::Decode { .. }
        | OperationError::TooLarge { .. }
        | OperationError::InvalidHeader { .. }
        | OperationError::TransformRetry(_)
        | OperationError::Parameters { .. }
        | OperationError::RejectedFields { .. }
        | OperationError::Anthropic { .. }) => raised(
            Raised::InternalServer,
            kept.to_string(),
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        ),
    }
}
// exception_type's mapping of a raw provider failure, keyed on its status and its text
pub(crate) fn classified(
    error_str: String,
    status_code: Option<u16>,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> CallFailure {
    let raw = RawException {
        model: facts.model.to_owned(),
        custom_llm_provider: facts.provider.to_owned(),
        error_str,
        status_code,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::new(),
    };
    CallFailure::Class(Box::new(dispatch(
        &DispatchInput::Raw(Box::new(raw)),
        redactor,
    )))
}
