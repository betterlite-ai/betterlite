use betterlite_models::controls::json_value::JsonValue;
use thiserror::Error;

use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::provider_config::{
    ASYNC_PROMPT_MERGED_PARAMS, AsyncPromptManagementOutcome,
};

#[derive(Debug, Error)]
#[error("the async prompt outcome is malformed: {detail}")]
pub struct AsyncOutcomeError {
    pub detail: String,
}

// LiteLLM's one-shot async outcome: popped from kwargs, never run through a sync hook
pub fn take_async_outcome(
    kwargs: &mut ParameterMap,
) -> Result<Option<AsyncPromptManagementOutcome>, AsyncOutcomeError> {
    let Some(value) = kwargs.shift_remove(ASYNC_PROMPT_MERGED_PARAMS) else {
        return Ok(None);
    };
    parse_async_outcome(value).map(Some)
}

fn parse_async_outcome(
    value: JsonValue,
) -> Result<AsyncPromptManagementOutcome, AsyncOutcomeError> {
    let JsonValue::Object(mut fields) = value else {
        return Err(AsyncOutcomeError {
            detail: String::from("the outcome is not an object"),
        });
    };
    let Some(JsonValue::Object(merged_optional_params)) =
        fields.shift_remove("merged_optional_params")
    else {
        return Err(AsyncOutcomeError {
            detail: String::from("merged_optional_params is not an object"),
        });
    };
    let deployment_model_info = match fields.shift_remove("deployment_model_info") {
        Some(JsonValue::Null) | None => None,
        Some(value) => Some(value),
    };
    Ok(AsyncPromptManagementOutcome {
        merged_optional_params,
        deployment_model_info,
    })
}
