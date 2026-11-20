use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::vector_search_execution::error::SearchError;
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_write::error::WriteError;

#[derive(Debug, Error)]
pub enum VectorStoreSearchError {
    #[error("Vector store {operation} is not supported for {provider}")]
    Unsupported {
        operation: &'static str,
        provider: String,
    },
    #[error("{name} is not a valid LlmProviders")]
    InvalidProvider { name: String },
    #[error(
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model={model}"
    )]
    UnresolvedProvider { model: String },
    #[error("Invalid direct vector store embedding executor")]
    InvalidEmbeddingExecutor,
    #[error("the {field} field must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error("{source}")]
    Search {
        provider: String,
        #[source]
        source: SearchError,
    },
    #[error("{source}")]
    Create {
        provider: String,
        #[source]
        source: CreateReadError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error("the timeout is not a number of seconds")]
    Timeout(#[source] TryFromFloatSecsError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

// What a caller sees: the SDK exception exception_type builds, or the bare TypeError raised before its try block
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdkRaise {
    Exception(SdkFailure),
    TypeError(TypeFailure),
}

// _direct_vector_store_embedding_executor raises this before the try, so the client decorator re-raises it unchanged: no status, type, param or code
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeFailure {
    pub message: String,
}

// The exception the SDK raises: its class, status and the type/param/code attributes it carries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkFailure {
    pub class: String,
    pub status: u16,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(error: &VectorStoreSearchError, redactor: &Redactor) -> SdkRaise {
    // the direct executor check runs before the try block, so its TypeError never reaches exception_type
    if matches!(error, VectorStoreSearchError::InvalidEmbeddingExecutor) {
        return SdkRaise::TypeError(TypeFailure {
            message: error.to_string(),
        });
    }
    let (provider, status, class, message) = raised(error);
    let projected = dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: message.clone(),
            status_code: status,
            status_code_is_synthesized: false,
            message: Some(message.clone()),
            body: None,
            exception_type: class,
        })),
        redactor,
    );
    let metadata = projected.metadata();
    SdkRaise::Exception(SdkFailure {
        class: class_name(&projected).to_owned(),
        status: metadata.status_code,
        kind: metadata.error_type,
        param: metadata.param,
        code: metadata
            .openai_code
            .map_or(WireField::Null, WireField::Text),
        message,
    })
}

// The provider, its status, the exception class exception_type saw and the text it mapped
fn raised(error: &VectorStoreSearchError) -> (&str, Option<u16>, String, String) {
    let (provider, status) = match error {
        VectorStoreSearchError::Search { provider, source } => {
            (provider.as_str(), search_status(source))
        }
        VectorStoreSearchError::Create { provider, source } => {
            (provider.as_str(), create_status(source))
        }
        VectorStoreSearchError::Unsupported { provider, .. } => (provider.as_str(), None),
        VectorStoreSearchError::InvalidProvider { .. }
        | VectorStoreSearchError::UnresolvedProvider { .. }
        | VectorStoreSearchError::InvalidEmbeddingExecutor
        | VectorStoreSearchError::Field { .. }
        | VectorStoreSearchError::Logging(_)
        | VectorStoreSearchError::Timeout(_)
        | VectorStoreSearchError::Encode(_) => ("", None),
    };
    // an SDK status error keeps its status and lets exception_type read the class from the status
    let class = if status.is_some() {
        String::new()
    } else {
        String::from("ValueError")
    };
    (provider, status, class, error.to_string())
}

const fn search_status(source: &SearchError) -> Option<u16> {
    if let SearchError::Provider(exception) = source {
        return Some(exception.status_code);
    }
    None
}

const fn create_status(source: &CreateReadError) -> Option<u16> {
    match source {
        CreateReadError::Write(WriteError::Provider(exception)) => Some(exception.status_code),
        CreateReadError::Write(
            WriteError::Client(_)
            | WriteError::Logging(_)
            | WriteError::Segment(_)
            | WriteError::Url(_)
            | WriteError::RequestUrl { .. }
            | WriteError::HeaderName { .. }
            | WriteError::HeaderValue { .. }
            | WriteError::Encode(_)
            | WriteError::Reply(_),
        )
        | CreateReadError::Capability(_) => None,
    }
}

const fn class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::ContextWindowExceeded(_) => "ContextWindowExceededError",
        ErrorClass::ContentPolicyViolation(_) => "ContentPolicyViolationError",
        ErrorClass::ServiceUnavailable(_) => "ServiceUnavailableError",
        ErrorClass::BadGateway(_) => "BadGatewayError",
        ErrorClass::InternalServer(_) => "InternalServerError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::RateLimit(_) => "RateLimitError",
        ErrorClass::ApiConnection(_) => "APIConnectionError",
        ErrorClass::ApiResponseValidation(_) => "APIResponseValidationError",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::Api(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => "APIError",
    }
}
