use betterlite_config::sections::guardrail_params::base::BaseLitellmParams;
use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};

// LiteLLM dumps a LitellmParams instance with model_dump(exclude_none=True), which drops its own null fields and leaves nulls inside plain dicts
pub(super) fn config_dump(params: &LitellmParams) -> Result<JsonObject, GuardrailParamsError> {
    let dump = stored_dump(params)?;
    Ok(without_none(&dump))
}

// a database row or a raw config item is a dict as written, so its null fields stay
pub(super) fn stored_dump(params: &LitellmParams) -> Result<JsonObject, GuardrailParamsError> {
    let bytes =
        serde_json::to_vec(params).map_err(|source| GuardrailParamsError::Encode { source })?;
    let dump: JsonObject =
        serde_json::from_slice(&bytes).map_err(|source| GuardrailParamsError::Encode { source })?;
    Ok(dump)
}

// BaseLitellmParams.model_validate(masked), null when the masked map is empty
pub(super) fn validated(
    masked: &JsonObject,
) -> Result<Option<BaseLitellmParams>, GuardrailParamsError> {
    if masked.is_empty() {
        return Ok(None);
    }
    base_params(masked).map(Some)
}

// v1 validates an empty dict too, which yields the all-null default model
pub(super) fn base_params(masked: &JsonObject) -> Result<BaseLitellmParams, GuardrailParamsError> {
    let opaque = opaque_params(masked)?;
    BaseLitellmParams::from_map(opaque.0)
}

pub(super) fn opaque_params(values: &JsonObject) -> Result<OpaqueParams, GuardrailParamsError> {
    let bytes =
        serde_json::to_vec(values).map_err(|source| GuardrailParamsError::Encode { source })?;
    serde_json::from_slice(&bytes).map_err(|source| GuardrailParamsError::Encode { source })
}

fn without_none(object: &JsonObject) -> JsonObject {
    let mut kept = JsonObject::with_capacity(object.len());
    for (key, value) in object {
        if matches!(value, JsonValue::Null) {
            continue;
        }
        kept.insert(key.clone(), value.clone());
    }
    kept
}
