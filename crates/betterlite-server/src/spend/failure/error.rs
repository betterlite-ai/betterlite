use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use betterlite_logging::fields::payload::records::ErrorInformation;
use betterlite_logging::logging_metadata::error_info::{ExceptionLogging, get_error_information};
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_logging::spend_sanitize::errors::sanitize_error_information;
use betterlite_logging::spend_sanitize::value::SpendValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

const TRACEBACK: &str = "traceback";

pub struct FailureError<'input> {
    pub logging: ExceptionLogging<'input>,
    pub model_not_found: Option<&'input ProxyModelNotFoundError>,
    pub traceback: Option<&'input str>,
    pub policy: SpendSanitizePolicy,
    pub suppress_traceback: bool,
}

#[derive(Debug, Error)]
pub enum FailureErrorInformationError {
    #[error("the error information could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// get_error_information, the suppressed traceback key, then _sanitize_error_information_for_spend_logs.
pub(crate) fn failure_error_information(
    error: &FailureError<'_>,
) -> Result<Json, FailureErrorInformationError> {
    let information = get_error_information(&error.logging, error.traceback);
    let mut fields = encoded_fields(&information)?;
    if error.suppress_traceback {
        fields.remove(TRACEBACK);
    }
    let value = SpendValue::from(&Json::Object(fields));
    let SpendValue::Mapping(mapping) = value else {
        return Ok(Json::Null);
    };
    let safe_message = error
        .model_not_found
        .map(|exception| exception.spend_log_error_message.as_str());
    Ok(sanitize_error_information(
        &mapping,
        &error.policy,
        safe_message,
    ))
}

fn encoded_fields(
    information: &ErrorInformation,
) -> Result<JsonObject, FailureErrorInformationError> {
    let encoded = serde_json::to_string(information)
        .map_err(|source| FailureErrorInformationError::Encode { source })?;
    serde_json::from_str(&encoded).map_err(|source| FailureErrorInformationError::Encode { source })
}
