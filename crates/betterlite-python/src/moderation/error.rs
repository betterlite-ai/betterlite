use super::convert::{ConvertError, NativeTextErrorPayload};
use crate::completion::error::SessionError as CompletionSessionError;
use betterlite_core::operation::moderation::route_error;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::moderation_execution::ModerationError;
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_transport::error::{ErrorStatus, TextCompletionError};
use betterlite_openai::text_completion_transport::sdk::status_error;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::Redactor;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};

create_exception!(betterlite_native, NativeTextError, PyException);

pub(crate) struct CallFacts<'facts> {
    pub(crate) model: &'facts str,
    pub(crate) provider: &'facts str,
}

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

// a redaction pattern that will not compile leaves the module unusable, not the request wrong
impl From<RedactionError> for CallFailure {
    fn from(source: RedactionError) -> Self {
        raised(Raised::InternalServer, source.to_string(), None, None)
    }
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
            SessionError::Json(_) | SessionError::Completion(_) | SessionError::Redact { .. } => {
                PyRuntimeError::new_err(message)
            }
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

fn refused(kind: Raised, message: String, facts: &CallFacts<'_>) -> CallFailure {
    raised(
        kind,
        message,
        Some(facts.model.to_owned()),
        Some(facts.provider.to_owned()),
    )
}

pub(crate) fn convert_failure(error: &ConvertError, facts: &CallFacts<'_>) -> CallFailure {
    let kind = match error {
        ConvertError::Prompt => Raised::Plain,
        ConvertError::Unsupported { .. } => Raised::UnsupportedParams,
        ConvertError::Json { .. } | ConvertError::Encode { .. } | ConvertError::Body { .. } => {
            Raised::BadRequest
        }
    };
    refused(kind, error.to_string(), facts)
}

pub(crate) enum RawSource<'failure> {
    Completion(&'failure CompletionFailure),
    Stream(&'failure TextCompletionError),
    Moderation(&'failure ModerationError),
}

pub(crate) fn provider_failure(
    source: &RawSource<'_>,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> CallFailure {
    let (message, status, body) = fields(source, redactor);
    let raw = RawException {
        model: facts.model.to_owned(),
        custom_llm_provider: facts.provider.to_owned(),
        error_str: message,
        status_code: status,
        status_code_is_synthesized: false,
        message: None,
        body: Some(body),
        exception_type: String::new(),
    };
    CallFailure(Box::new(dispatch(
        &DispatchInput::Raw(Box::new(raw)),
        redactor,
    )))
}

fn fields(source: &RawSource<'_>, redactor: &Redactor) -> (String, Option<u16>, RawExceptionBody) {
    match source {
        RawSource::Completion(error) => (
            error.message.clone(),
            Some(error.status),
            RawExceptionBody {
                kind: error.error_type.clone(),
                param: error.param.clone(),
                code: None,
                message: None,
                error: None,
            },
        ),
        RawSource::Stream(error) => (
            error.message.clone(),
            stream_status(error),
            RawExceptionBody::default(),
        ),
        RawSource::Moderation(error) => {
            let mut failure = route_error(error, redactor);
            // the SDK's exception carries the provider body's text, which the route's message drops
            if let ModerationError::Provider { response } = error {
                let sdk = status_error(response.status, response.headers.clone(), &response.body);
                failure.message = redactor.redact_internal_details(&sdk.to_string());
            }
            (
                failure.message,
                Some(failure.status),
                RawExceptionBody {
                    kind: failure.error_type,
                    param: failure.param,
                    code: failure.code,
                    message: None,
                    error: None,
                },
            )
        }
    }
}

// The moderation route's own status, type and param: a missing key is the openai client's bare Exception.
pub(crate) fn moderation_failure(
    error: &ModerationError,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> CallFailure {
    if matches!(error, ModerationError::MissingApiKey) {
        let failure = route_error(error, redactor);
        return refused(Raised::Plain, failure.message, facts);
    }
    provider_failure(&RawSource::Moderation(error), facts, redactor)
}

const fn stream_status(error: &TextCompletionError) -> Option<u16> {
    match &error.status {
        ErrorStatus::Missing | ErrorStatus::Null => None,
        ErrorStatus::Code(code) => Some(*code),
    }
}
