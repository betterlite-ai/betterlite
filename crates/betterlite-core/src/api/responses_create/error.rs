use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::headers::affinity::AffinityError;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::assistant_records::error::class_name;
use crate::api::response_id_traversal::response_id::ResponseRewriteError;
use crate::api::responses_create_http::error::CreateError as HttpCreateError;
use crate::api::responses_create_http::error::ParamError as HttpParamError;
use crate::api::responses_parameters::error::{
    ResponsesParametersError, UNSUPPORTED_PARAMS_STATUS,
};
use crate::api::responses_provider_selection::error::{
    ENCRYPTED_TASK_MESSAGE, compatibility_failure,
};
use crate::api::responses_provider_selection::file_ids::FileMappingError;
use crate::api::responses_provider_selection::prompting::PromptError;
use crate::api::responses_provider_selection::provider_config::CompatibilityFailure;

// Every provider config the factory yields reports parameter failures through this one type
#[derive(Debug, Error)]
pub enum CreateParamError {
    #[error(transparent)]
    Params(#[from] ResponsesParametersError),
}

// A dispatch port's own refusal, which the create wrapper reports as the call's failure
#[derive(Debug, Error)]
#[error("{message}")]
pub struct InterceptionError {
    pub message: String,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct BridgeError {
    pub message: String,
}

#[derive(Debug, Error)]
pub enum CreateError {
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Prompt(#[from] PromptError),
    #[error(transparent)]
    Param(#[from] CreateParamError),
    #[error(transparent)]
    Params(#[from] HttpParamError),
    #[error(transparent)]
    Affinity(#[from] AffinityError),
    #[error(transparent)]
    FileMapping(#[from] FileMappingError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Request(#[from] HttpCreateError),
    #[error(transparent)]
    Rewrite(#[from] ResponseRewriteError),
    #[error(transparent)]
    Interception(#[from] InterceptionError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error("Encrypted task classification requires a compatible native Responses deployment")]
    Compatibility(CompatibilityFailure),
    #[error("custom_llm_provider is required but passed as None")]
    MissingProvider,
    #[error("RESPONSES is not supported for {provider}")]
    Unsupported { provider: String },
    #[error("the timeout {seconds} is not a duration")]
    InvalidTimeout { seconds: f64 },
    #[error("{field} must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error("litellm_metadata could not be read")]
    Metadata(#[source] serde_json::Error),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

// The SDK exception a caller sees; a local failure carries no body, so type, param and code stay absent
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateFailure {
    pub class: &'static str,
    pub status: u16,
    pub kind: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

impl CreateError {
    #[must_use]
    pub fn sdk_failure(&self, provider: &str, model: &str, redactor: &Redactor) -> CreateFailure {
        if let Self::Param(CreateParamError::Params(
            ResponsesParametersError::UnsupportedParams { .. },
        )) = self
        {
            return CreateFailure {
                class: "UnsupportedParamsError",
                status: UNSUPPORTED_PARAMS_STATUS,
                kind: None,
                param: None,
                code: None,
                message: redactor.redact_string(&self.to_string()),
            };
        }
        let class = self.error_class(provider, model, redactor);
        CreateFailure {
            class: class_name(&class),
            status: class.metadata().status_code,
            kind: None,
            param: None,
            code: None,
            message: redactor.redact_string(&self.failure_facts().0),
        }
    }

    #[must_use]
    pub fn error_class(&self, provider: &str, model: &str, redactor: &Redactor) -> ErrorClass {
        match self {
            Self::Compatibility(failure) => {
                compatibility_failure(*failure, model, Some(provider), redactor)
            }
            Self::Affinity(error) => {
                let context = ErrorContext {
                    message: format!("litellm.BadRequestError: {error}"),
                    model: Some(model.to_owned()),
                    provider: Some(provider.to_owned()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                };
                ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, Some(400))))
            }
            Self::Resolution(_)
            | Self::Prompt(_)
            | Self::Param(_)
            | Self::Params(_)
            | Self::FileMapping(_)
            | Self::Timeout(_)
            | Self::Logging(_)
            | Self::Request(_)
            | Self::Rewrite(_)
            | Self::Interception(_)
            | Self::Bridge(_)
            | Self::MissingProvider
            | Self::Unsupported { .. }
            | Self::InvalidTimeout { .. }
            | Self::Field { .. }
            | Self::Metadata(_)
            | Self::Encode(_) => {
                let (error_str, status) = self.failure_facts();
                let exception = RawException {
                    model: model.to_owned(),
                    custom_llm_provider: provider.to_owned(),
                    error_str: error_str.clone(),
                    status_code: status,
                    status_code_is_synthesized: matches!(
                        self,
                        Self::Request(HttpCreateError::Provider(error))
                            if error.status_code_is_synthesized
                    ),
                    message: Some(error_str),
                    body: None,
                    exception_type: String::new(),
                };
                dispatch(&DispatchInput::Raw(Box::new(exception)), redactor)
            }
        }
    }

    // exception_type() reads the provider's own status and text; a local failure has neither
    fn failure_facts(&self) -> (String, Option<u16>) {
        match self {
            Self::Affinity(error) => (format!("litellm.BadRequestError: {error}"), None),
            Self::Compatibility(_) => (String::from(ENCRYPTED_TASK_MESSAGE), None),
            Self::Request(HttpCreateError::Provider(error)) => {
                (error.message.clone(), Some(error.status_code))
            }
            Self::Resolution(_)
            | Self::Prompt(_)
            | Self::Param(_)
            | Self::Params(_)
            | Self::FileMapping(_)
            | Self::Timeout(_)
            | Self::Logging(_)
            | Self::Request(_)
            | Self::Rewrite(_)
            | Self::Interception(_)
            | Self::Bridge(_)
            | Self::MissingProvider
            | Self::Unsupported { .. }
            | Self::InvalidTimeout { .. }
            | Self::Field { .. }
            | Self::Metadata(_)
            | Self::Encode(_) => (self.to_string(), None),
        }
    }
}
