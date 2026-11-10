use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::types::SuccessError;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;

pub const STANDARD_LOGGING_OBJECT: &str = "standard_logging_object";

pub fn set_standard_logging_object(
    state: &mut LoggingState,
    payload: Option<&StandardLoggingPayload>,
) -> Result<(), SuccessError> {
    let Some(stored) = payload else {
        state
            .model_call_details
            .shift_remove(STANDARD_LOGGING_OBJECT);
        return Ok(());
    };
    state.model_call_details.insert(
        STANDARD_LOGGING_OBJECT.to_owned(),
        Json::Object(payload_to_object(stored)?),
    );
    Ok(())
}

pub fn standard_logging_object(
    state: &LoggingState,
) -> Result<Option<StandardLoggingPayload>, SuccessError> {
    let Some(value) = state.model_call_details.get(STANDARD_LOGGING_OBJECT) else {
        return Ok(None);
    };
    let Json::Object(fields) = value else {
        return Err(SuccessError::PayloadShape);
    };
    payload_from_object(fields).map(Some)
}

pub(super) fn payload_to_object(
    payload: &StandardLoggingPayload,
) -> Result<IndexMap<String, Json>, SuccessError> {
    let text =
        serde_json::to_string(payload).map_err(|source| SuccessError::PayloadJson { source })?;
    let value: Json =
        serde_json::from_str(&text).map_err(|source| SuccessError::PayloadJson { source })?;
    let Json::Object(fields) = value else {
        return Err(SuccessError::PayloadShape);
    };
    Ok(fields)
}

pub(super) fn payload_from_object(
    fields: &IndexMap<String, Json>,
) -> Result<StandardLoggingPayload, SuccessError> {
    let text =
        serde_json::to_string(fields).map_err(|source| SuccessError::PayloadJson { source })?;
    serde_json::from_str(&text).map_err(|source| SuccessError::PayloadJson { source })
}
