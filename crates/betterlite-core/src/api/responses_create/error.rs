use crate::api::assistant_records::error::class_name;
use crate::api::response_id_traversal::response_id::ResponseRewriteError;
use crate::api::responses_compact_call::error::CompactFailure;
use crate::api::responses_create_http::error::CreateError as HttpCreateError;
use crate::api::responses_create_http::error::ParamError as HttpParamError;
use crate::api::responses_parameters::error::{
    ResponsesParametersError, UNSUPPORTED_PARAMS_STATUS,
};
use crate::api::responses_provider_selection::error::compatibility_failure;
use crate::api::responses_provider_selection::file_ids::FileMappingError;
use crate::api::responses_provider_selection::prompting::PromptError;
use crate::api::responses_provider_selection::provider_config::CompatibilityFailure;
use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::headers::affinity::AffinityError;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;
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
    Param(#[from] ResponsesParametersError),
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
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}
pub type CreateFailure = CompactFailure;
impl CreateError {
    #[must_use]
    pub fn sdk_failure(&self, provider: &str, model: &str, redactor: &Redactor) -> CreateFailure {
        if let Self::Param(ResponsesParametersError::UnsupportedParams { .. }) = self {
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
            message: redactor.redact_string(&class_message(&class)),
        }
    }
    #[must_use]
    pub fn error_class(&self, provider: &str, model: &str, redactor: &Redactor) -> ErrorClass {
        if let Self::Compatibility(failure) = self {
            return compatibility_failure(*failure, model, Some(provider), redactor);
        }
        if let Self::Affinity(error) = self {
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
            return ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
                context,
                Some(400),
            )));
        }
        let (error_str, status) = if let Self::Request(HttpCreateError::Provider(error)) = self {
            (error.message.clone(), Some(error.status_code))
        } else {
            (self.to_string(), None)
        };
        let synthesized = matches!(
            self,
            Self::Request(HttpCreateError::Provider(error)) if error.status_code_is_synthesized
        );
        let exception = RawException {
            model: model.to_owned(),
            custom_llm_provider: provider.to_owned(),
            error_str: error_str.clone(),
            status_code: status,
            status_code_is_synthesized: synthesized,
            message: Some(error_str),
            body: None,
            exception_type: String::new(),
        };
        dispatch(&DispatchInput::Raw(Box::new(exception)), redactor)
    }
}
