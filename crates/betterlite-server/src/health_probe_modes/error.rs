use base64::DecodeError;
use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_core::api::request_principal_metadata::error::RequestPrincipalMetadataError;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use std::error::Error;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum HealthProbeError {
    #[error("the health check parameter {key} cannot be converted into the call JSON")]
    ParameterValue {
        key: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "the {mode} health check cannot add {key}, because the model parameters already set it"
    )]
    RepeatedParameter {
        key: &'static str,
        mode: &'static str,
    },
    #[error(
        "Unable to health check wildcard model for provider {provider}. Add a model on your config.yaml or contribute here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json"
    )]
    NoCheapModel { provider: String },
    #[error("the evaluation health check field {field} is invalid")]
    EvaluationPayload {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("no ocr health check document is available for {model} through {custom_llm_provider}")]
    OcrDocumentUnavailable {
        model: String,
        custom_llm_provider: String,
    },
    #[error("the bundled health check image could not be decoded")]
    HealthImage {
        #[source]
        source: DecodeError,
    },
    #[error("the health check logging object could not be updated")]
    Logging {
        #[source]
        source: LoggingStateError,
    },
    #[error("the internal health principal could not be built")]
    Principal {
        #[source]
        source: AuthError,
    },
    #[error("the internal health principal could not be encoded")]
    PrincipalEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the health check tracking metadata could not be recorded")]
    Tracking {
        #[source]
        source: RequestPrincipalMetadataError,
    },
    #[error("the health check tracking metadata was not written as an object")]
    TrackingMetadata,
    // a failing provider call keeps its class, its message and its own error
    #[error("{message}")]
    Backend {
        class: ErrorClass,
        message: String,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}
