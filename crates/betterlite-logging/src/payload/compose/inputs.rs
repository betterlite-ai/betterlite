use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::response::{ExtractedResponse, extract};
use crate::logging_metadata::decode::decode;
use crate::logging_metadata::params::LitellmParams;
use crate::logging_metadata::scrub::scrub_sensitive_metadata;
use crate::logging_state::LoggingState;
use crate::payload::compose::PayloadRequest;
use crate::payload::error::PayloadError;
use crate::payload::response::init_response_object;

// The litellm_params keys StandardLoggingPayloadSetup reads; everything else stays out of the typed view
const LITELLM_PARAM_KEYS: [&str; 7] = [
    "metadata",
    "litellm_metadata",
    "litellm_session_id",
    "litellm_trace_id",
    "prompt_id",
    "prompt_variables",
    "_langfuse_masking_function",
];

pub(crate) fn litellm_params(state: &LoggingState) -> Result<LitellmParams, PayloadError> {
    let filtered: IndexMap<String, Json> = state
        .litellm_params
        .iter()
        .filter(|(key, _value)| LITELLM_PARAM_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let decoded: LitellmParams =
        decode("litellm_params", filtered).map_err(|source| PayloadError::Decode {
            field: "litellm_params",
            source,
        })?;
    Ok(scrub_sensitive_metadata(decoded))
}

pub(crate) fn optional_text(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PayloadError::FieldType { field }),
    }
}

pub(crate) fn extracted_response(request: &PayloadRequest<'_>) -> ExtractedResponse {
    extract(
        init_response_object(request.init_response_obj),
        super::error_info::exception_headers(request.original_exception),
    )
}
