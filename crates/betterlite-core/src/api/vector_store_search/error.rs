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
    Timeout(#[source] std::time::TryFromFloatSecsError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

// The exception the SDK raises: its class, status and the type/param/code attributes it carries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkFailure {
    pub class: String,
    pub status: Option<u16>,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(error: &VectorStoreSearchError, redactor: &Redactor) -> SdkFailure {
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
    SdkFailure {
        class: class_name(&projected).to_owned(),
        status: Some(metadata.status_code),
        kind: metadata.error_type,
        param: metadata.param,
        code: metadata
            .openai_code
            .map_or(WireField::Null, WireField::Text),
        message,
    }
}

// The provider, its status, the exception class exception_type saw and the text it mapped
fn raised(error: &VectorStoreSearchError) -> (&str, Option<u16>, String, String) {
    match error {
        VectorStoreSearchError::Unsupported { provider, .. } => (
            provider,
            None,
            String::from("ValueError"),
            error.to_string(),
        ),
        VectorStoreSearchError::Search { provider, source } => match source {
            SearchError::Provider(exception) => (
                provider,
                Some(exception.status_code),
                String::new(),
                error.to_string(),
            ),
            SearchError::Client(_)
            | SearchError::Logging(_)
            | SearchError::Request(_)
            | SearchError::RequestUrl { .. }
            | SearchError::HeaderName { .. }
            | SearchError::HeaderValue { .. }
            | SearchError::Encode(_)
            | SearchError::Field { .. }
            | SearchError::Signing(_)
            | SearchError::Direct(_) => (
                provider,
                None,
                String::from("ValueError"),
                error.to_string(),
            ),
        },
        VectorStoreSearchError::Create { provider, source } => match source {
            CreateReadError::Write(WriteError::Provider(exception)) => (
                provider,
                Some(exception.status_code),
                String::new(),
                error.to_string(),
            ),
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
            | CreateReadError::Capability(_) => (
                provider,
                None,
                String::from("ValueError"),
                error.to_string(),
            ),
        },
        VectorStoreSearchError::InvalidProvider { .. }
        | VectorStoreSearchError::UnresolvedProvider { .. }
        | VectorStoreSearchError::InvalidEmbeddingExecutor
        | VectorStoreSearchError::Field { .. }
        | VectorStoreSearchError::Logging(_)
        | VectorStoreSearchError::Timeout(_)
        | VectorStoreSearchError::Encode(_) => {
            ("", None, String::from("ValueError"), error.to_string())
        }
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
