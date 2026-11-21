use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::service::Operation;
use crate::api::vector_store_http_write::error::WriteError;

// exception_type() maps every write failure with model None and the provider the call resolved to
const MODEL: &str = "";

#[derive(Debug, Error)]
pub enum VectorStoreWriteError {
    #[error("'{provider}' is not a valid LlmProviders")]
    InvalidProvider { provider: String },
    #[error("Vector store {} is not supported for {provider}", operation.label())]
    Unsupported {
        operation: Operation,
        provider: String,
    },
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    // the Json-to-UpdateParams bridge; a failure here is a local ValueError, not a provider answer
    #[error("the vector store parameters are not valid")]
    Params(#[source] serde_json::Error),
    #[error("the request timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // B2402's write service, whose provider errors carry the status exception_type() maps by
    #[error(transparent)]
    Write(#[from] WriteError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct VectorStoreWriteFailure {
    #[source]
    pub error: VectorStoreWriteError,
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

impl VectorStoreWriteFailure {
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
    error: VectorStoreWriteError,
    provider: &str,
    redactor: &Redactor,
) -> VectorStoreWriteFailure {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(raw_exception(&error, provider))),
        redactor,
    );
    VectorStoreWriteFailure { error, class }
}

fn raw_exception(error: &VectorStoreWriteError, provider: &str) -> RawException {
    let (message, status_code, synthesized, exception_type) = match error {
        VectorStoreWriteError::Write(WriteError::Provider(source)) => (
            source.message.clone(),
            Some(source.status_code),
            source.status_code_is_synthesized,
            String::new(),
        ),
        VectorStoreWriteError::InvalidProvider { .. }
        | VectorStoreWriteError::Unsupported { .. }
        | VectorStoreWriteError::Resolution(_)
        | VectorStoreWriteError::Params(_)
        | VectorStoreWriteError::Timeout { .. }
        | VectorStoreWriteError::Logging(_)
        | VectorStoreWriteError::Write(_) => {
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
