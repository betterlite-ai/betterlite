use betterlite_config::sections::guardrail_params::base::BaseLitellmParams;
use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};

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
    Ok(BaseLitellmParams::from_map(opaque_params(masked)?.0))
}

pub(super) fn opaque_params(values: &JsonObject) -> Result<OpaqueParams, GuardrailParamsError> {
    let bytes =
        serde_json::to_vec(values).map_err(|source| GuardrailParamsError::Encode { source })?;
    serde_json::from_slice(&bytes).map_err(|source| GuardrailParamsError::Encode { source })
}

pub(super) fn without_none(object: &JsonObject) -> JsonObject {
    let mut kept = JsonObject::with_capacity(object.len());
    for (key, value) in object {
        if matches!(value, JsonValue::Null) {
            continue;
        }
        kept.insert(key.clone(), value.clone());
    }
    kept
}
