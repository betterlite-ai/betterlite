use super::convert::{ConvertError, NativeTextErrorPayload, UNMAPPED_PROMPT};
use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_core::operation::moderation::route_error;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::moderation_execution::ModerationError;
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_transport::error::{ErrorStatus, TextCompletionError};
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};

create_exception!(betterlite_native, NativeTextError, PyException);

// the identity every error carries back to Python
pub(crate) struct CallFacts<'facts> {
    pub(crate) model: &'facts str,
    pub(crate) provider: &'facts str,
}

// One failed call, as the LiteLLM exception class main.py re-raises.
#[derive(Debug)]
pub(crate) struct CallFailure(pub(crate) Box<ErrorClass>);

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
        // completion's parameter check raises UnsupportedParamsError for a kwarg the provider cannot take
        Raised::UnsupportedParams => ErrorClass::UnsupportedParams(Box::new(context)),
        // LiteLLM raises a bare Exception where no LiteLLM class fits, which carries neither a status nor a body
        Raised::Plain => ErrorClass::Unclassified(Box::new(context)),
    };
    CallFailure(Box::new(class))
}

pub(crate) fn class_error(class: &ErrorClass) -> PyErr {
    let payload = NativeTextErrorPayload::from_class(class);
    match serde_json::to_string(&payload) {
        Ok(text) => NativeTextError::new_err(text),
        Err(_encoding) => PyRuntimeError::new_err(payload.message),
    }
}

impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        let message = error.to_string();
        match error {
            SessionError::Json(ConvertError::Json { .. }) => PyValueError::new_err(message),
            SessionError::Json(
                ConvertError::Encode { .. }
                | ConvertError::Body { .. }
                | ConvertError::Unsupported { .. }
                | ConvertError::Prompt,
            )
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

// A request the native method must refuse before a provider runs: the prompt shape is main.py's bare Exception.
pub(crate) fn convert_failure(error: ConvertError, facts: &CallFacts<'_>) -> CallFailure {
    if matches!(error, ConvertError::Prompt) {
        return raised(
            Raised::Plain,
            String::from(UNMAPPED_PROMPT),
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        );
    }
    if matches!(error, ConvertError::Unsupported { .. }) {
        return raised(
            Raised::UnsupportedParams,
            error.to_string(),
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        );
    }
    raised(
        Raised::BadRequest,
        error.to_string(),
        Some(facts.model.to_owned()),
        Some(facts.provider.to_owned()),
    )
}

// the parts of a provider failure that pick its status, type, param and code
struct RawFailure {
    message: String,
    status: Option<u16>,
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

// Every prepare, execute and decode failure, as the LiteLLM exception class main.py re-raises.
pub(crate) fn completion_failure(
    failure: &CompletionFailure,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> CallFailure {
    raw_failure(
        RawFailure {
            message: failure.message.clone(),
            status: Some(failure.status),
            error_type: failure.error_type.clone(),
            param: failure.param.clone(),
            code: None,
        },
        facts,
        redactor,
    )
}

// A provider stream that stops with a chunk it cannot shape
pub(crate) fn stream_failure(
    error: &TextCompletionError,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> CallFailure {
    let status = match &error.status {
        ErrorStatus::Missing | ErrorStatus::Null => None,
        ErrorStatus::Code(code) => Some(*code),
    };
    raw_failure(
        RawFailure {
            message: error.message.clone(),
            status,
            error_type: None,
            param: None,
            code: None,
        },
        facts,
        redactor,
    )
}

// The moderation route's own status, type and param, mapped to LiteLLM's exception class.
pub(crate) fn moderation_failure(
    error: &ModerationError,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> CallFailure {
    let failure = route_error(error, redactor);
    if matches!(error, ModerationError::MissingApiKey) {
        // the openai client raises its own bare Exception when no key source is set
        return raised(
            Raised::Plain,
            failure.message,
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        );
    }
    raw_failure(
        RawFailure {
            message: failure.message,
            status: Some(failure.status),
            error_type: failure.error_type,
            param: failure.param,
            code: failure.code,
        },
        facts,
        redactor,
    )
}

// A mock timeout: the SDK mock entry's timeout error, raised before any provider runs.
pub(crate) fn mock_timeout_failure(facts: &CallFacts<'_>, redactor: &Redactor) -> CallFailure {
    let error = MockError::Timeout;
    raw_failure(
        RawFailure {
            message: error.to_string(),
            status: error.status(),
            error_type: None,
            param: None,
            code: None,
        },
        facts,
        redactor,
    )
}

// exception_type's mapping of a raw provider failure, keyed on its status and the body fields it answers with
fn raw_failure(failure: RawFailure, facts: &CallFacts<'_>, redactor: &Redactor) -> CallFailure {
    let raw = RawException {
        model: facts.model.to_owned(),
        custom_llm_provider: facts.provider.to_owned(),
        error_str: failure.message,
        status_code: failure.status,
        status_code_is_synthesized: false,
        message: None,
        body: Some(RawExceptionBody {
            kind: failure.error_type,
            param: failure.param,
            code: failure.code,
            message: None,
            error: None,
        }),
        exception_type: String::new(),
    };
    CallFailure(Box::new(dispatch(
        &DispatchInput::Raw(Box::new(raw)),
        redactor,
    )))
}
