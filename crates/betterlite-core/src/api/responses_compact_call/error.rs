use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::response_id_traversal::response_id::ResponseRewriteError;
use crate::api::responses_compact_http::error::CompactError;
use crate::api::responses_parameters::error::{
    ResponsesParametersError, UNSUPPORTED_PARAMS_STATUS,
};

// Every provider config the factory yields reports parameter failures through this one type
#[derive(Debug, Error)]
pub enum CompactParamError {
    #[error(transparent)]
    Params(#[from] ResponsesParametersError),
}

#[derive(Debug, Error)]
pub enum CompactResponsesError {
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error("custom_llm_provider is required but passed as None")]
    MissingProvider,
    #[error("COMPACT responses is not supported for {provider}")]
    Unsupported { provider: String },
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error(transparent)]
    Param(#[from] CompactParamError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Request(#[from] CompactError),
    #[error(transparent)]
    Rewrite(#[from] ResponseRewriteError),
    #[error("the timeout {seconds} is not a duration")]
    InvalidTimeout { seconds: f64 },
    #[error("litellm_metadata could not be read")]
    Metadata(#[source] serde_json::Error),
}

// The SDK exception a caller sees; _handle_error carries no body, so type, param and code stay absent
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactFailure {
    pub class: &'static str,
    pub status: u16,
    pub kind: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

impl CompactResponsesError {
    #[must_use]
    pub fn sdk_failure(&self, provider: &str, model: &str, redactor: &Redactor) -> CompactFailure {
        let message = redactor.redact_string(&self.to_string());
        if let Self::Param(CompactParamError::Params(
            ResponsesParametersError::UnsupportedParams { .. },
        )) = self
        {
            return CompactFailure {
                class: "UnsupportedParamsError",
                status: UNSUPPORTED_PARAMS_STATUS,
                kind: None,
                param: None,
                code: None,
                message,
            };
        }
        let class = self.error_class(provider, model, redactor);
        CompactFailure {
            class: sdk_class(&class),
            status: class.metadata().status_code,
            kind: None,
            param: None,
            code: None,
            message,
        }
    }

    fn error_class(&self, provider: &str, model: &str, redactor: &Redactor) -> ErrorClass {
        let (error_str, status) = self.failure_facts();
        let exception = RawException {
            model: model.to_owned(),
            custom_llm_provider: provider.to_owned(),
            error_str: error_str.clone(),
            status_code: status,
            status_code_is_synthesized: matches!(
                self,
                Self::Request(CompactError::Provider(error)) if error.status_code_is_synthesized
            ),
            message: Some(error_str),
            body: None,
            exception_type: String::new(),
        };
        dispatch(&DispatchInput::Raw(Box::new(exception)), redactor)
    }

    // exception_type() reads the provider's own status and text; a local failure has neither
    fn failure_facts(&self) -> (String, Option<u16>) {
        match self {
            Self::Request(CompactError::Provider(error)) => {
                (error.message.clone(), Some(error.status_code))
            }
            Self::Resolution(_)
            | Self::MissingProvider
            | Self::Unsupported { .. }
            | Self::Timeout(_)
            | Self::Param(_)
            | Self::Logging(_)
            | Self::Request(_)
            | Self::Rewrite(_)
            | Self::InvalidTimeout { .. }
            | Self::Metadata(_) => (self.to_string(), None),
        }
    }
}

// exception_type()'s local exception name for each family it can classify
const fn sdk_class(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::RateLimit(_) => "RateLimitError",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::ContextWindowExceeded(_) => "ContextWindowExceededError",
        ErrorClass::ContentPolicyViolation(_) => "ContentPolicyViolationError",
        ErrorClass::ServiceUnavailable(_) => "ServiceUnavailableError",
        ErrorClass::BadGateway(_) => "BadGatewayError",
        ErrorClass::InternalServer(_) => "InternalServerError",
        ErrorClass::Api(_) => "APIError",
        ErrorClass::ApiConnection(_) => "APIConnectionError",
        ErrorClass::ApiResponseValidation(_) => "APIResponseValidationError",
        ErrorClass::JsonSchemaValidation(_) => "JSONSchemaValidationError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::GuardrailRaised(_) => "GuardrailRaisedException",
        ErrorClass::MidStreamFallback(_) => "MidStreamFallbackError",
        ErrorClass::Unclassified(_) => "APIError",
    }
}
