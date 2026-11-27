use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::vector_file_http_write::error::FileWriteError;
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

// exception_type() maps every file write failure with model None and the provider the call resolved to
const MODEL: &str = "";

#[derive(Debug, Error)]
pub enum VectorFileWriteError {
    #[error("'{provider}' is not a valid LlmProviders")]
    InvalidProvider { provider: String },
    #[error("Vector store file {} is not supported for {provider}", label(*operation))]
    Unsupported {
        operation: Operation,
        provider: String,
    },
    #[error("{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    // the Json-to-record bridges; a failure here is a local ValueError, not a provider answer
    #[error("the vector store file parameters are not valid")]
    Params(#[source] serde_json::Error),
    // the effective timeout httpx expires before any request goes out
    #[error("the request timeout of {seconds} seconds expired before the request was sent")]
    Expired {
        operation: Operation,
        mode: ClientKind,
        seconds: f64,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Write(#[from] FileWriteError),
}

const fn label(operation: Operation) -> &'static str {
    match operation {
        Operation::Update => "update",
        Operation::Delete => "delete",
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct VectorFileWriteFailure {
    #[source]
    pub error: VectorFileWriteError,
    pub class: ErrorClass,
}

// the exception a Python caller catches, as exception_type() raises it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaisedException {
    pub class: &'static str,
    pub status: u16,
    pub kind: WireField,
    pub param: WireField,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Error)]
#[error("the mapped error class has no SDK exception")]
pub struct UnprojectedClass;

impl VectorFileWriteFailure {
    pub fn raised(&self) -> Result<RaisedException, UnprojectedClass> {
        let metadata = self.class.metadata();
        let (class, message) = classify(&self.class).ok_or(UnprojectedClass)?;
        Ok(RaisedException {
            class,
            status: metadata.status_code,
            kind: metadata.error_type,
            param: metadata.param,
            code: metadata.openai_code,
            message: message.to_owned(),
        })
    }
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(
    error: VectorFileWriteError,
    provider: &str,
    redactor: &Redactor,
) -> VectorFileWriteFailure {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(raw_exception(&error, provider))),
        redactor,
    );
    VectorFileWriteFailure { error, class }
}

fn raw_exception(error: &VectorFileWriteError, provider: &str) -> RawException {
    let (message, status_code, synthesized, exception_type) = match error {
        VectorFileWriteError::Write(FileWriteError::Provider(source)) => (
            source.message.clone(),
            Some(source.status_code),
            source.status_code_is_synthesized,
            String::new(),
        ),
        VectorFileWriteError::Expired {
            operation,
            mode,
            seconds,
        } => expired_exception(*operation, *mode, *seconds),
        VectorFileWriteError::InvalidProvider { .. }
        | VectorFileWriteError::Unsupported { .. }
        | VectorFileWriteError::FieldType { .. }
        | VectorFileWriteError::Params(_)
        | VectorFileWriteError::Logging(_)
        | VectorFileWriteError::Write(_) => {
            (error.to_string(), None, false, String::from("ValueError"))
        }
    };
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: synthesized,
        message: Some(message),
        body: None,
        exception_type,
    }
}

// An expired deadline: the catches raise litellm.Timeout's 408, and the async delete's missing catch gets _handle_error's synthesized 500
fn expired_exception(
    operation: Operation,
    mode: ClientKind,
    seconds: f64,
) -> (String, Option<u16>, bool, String) {
    if mode == ClientKind::Asynchronous && operation == Operation::Delete {
        return (
            String::from("timed out"),
            Some(500_u16),
            true,
            String::new(),
        );
    }
    let message = match mode {
        ClientKind::Sync => {
            format!("litellm.Timeout: Connection timed out after {seconds} seconds.")
        }
        ClientKind::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={seconds}, time taken=0.0 seconds"
        ),
    };
    (message, Some(408_u16), false, String::new())
}

// the name of the exception class exception_type() raises, and the message it carries
fn classify(class: &ErrorClass) -> Option<(&'static str, &str)> {
    match class {
        ErrorClass::Authentication(details) => Some(("AuthenticationError", &details.message)),
        ErrorClass::NotFound(details) => Some(("NotFoundError", &details.message)),
        ErrorClass::PermissionDenied(details) => Some(("PermissionDeniedError", &details.message)),
        ErrorClass::BadRequest(details) => Some(("BadRequestError", &details.message)),
        ErrorClass::UnprocessableEntity(details) => {
            Some(("UnprocessableEntityError", &details.message))
        }
        ErrorClass::ContextWindowExceeded(details) => {
            Some(("ContextWindowExceededError", &details.message))
        }
        ErrorClass::ContentPolicyViolation(details) => {
            Some(("ContentPolicyViolationError", &details.message))
        }
        ErrorClass::ServiceUnavailable(context) => {
            Some(("ServiceUnavailableError", &context.message))
        }
        ErrorClass::BadGateway(context) => Some(("BadGatewayError", &context.message)),
        ErrorClass::InternalServer(context) => Some(("InternalServerError", &context.message)),
        ErrorClass::Timeout(details) => Some(("Timeout", &details.context.message)),
        ErrorClass::RateLimit(details) => Some(("RateLimitError", &details.context.message)),
        ErrorClass::Api(details) => Some(("APIError", &details.context.message)),
        ErrorClass::ApiConnection(context) => Some(("APIConnectionError", &context.message)),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => None,
    }
}
