use crate::datadog_llm_obs::span_records::DDLLMObsError;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

// A status alone carries no details, so an error object exists only when error_information does
#[must_use]
pub fn error_info(payload: &StandardLoggingPayload) -> Option<DDLLMObsError> {
    if payload.status != LoggingStatus::Failure {
        return None;
    }
    let information = payload.error_information.as_ref()?;
    let message = information
        .error_message
        .as_ref()
        .and_then(Option::as_deref)
        .or(payload.error_str.as_deref())
        .unwrap_or("Unknown error");
    Some(DDLLMObsError {
        message: Some(message.to_owned()),
        stack: information
            .traceback
            .as_ref()
            .and_then(Option::as_deref)
            .map(str::to_owned),
        kind: information
            .error_class
            .as_ref()
            .and_then(Option::as_deref)
            .map(str::to_owned),
    })
}
