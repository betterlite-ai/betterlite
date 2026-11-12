use serde::de::DeserializeOwned;

use super::error::GuardrailParamsError;
use super::params::LitellmParams;
use super::routes::{ApplyGuardrailRequest, PatchGuardrailRequest};

pub(super) fn decode_fields<T>(bytes: &[u8]) -> Result<T, GuardrailParamsError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut deserializer)?;
    deserializer
        .end()
        .map_err(|_trailing| GuardrailParamsError::TrailingData)?;
    Ok(value)
}

pub fn decode_litellm_params(body: &[u8]) -> Result<LitellmParams, GuardrailParamsError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let params = serde_path_to_error::deserialize(&mut deserializer)?;
    deserializer
        .end()
        .map_err(|_trailing| GuardrailParamsError::TrailingData)?;
    Ok(params)
}

pub fn decode_patch_request(body: &[u8]) -> Result<PatchGuardrailRequest, GuardrailParamsError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let request = serde_path_to_error::deserialize(&mut deserializer)?;
    deserializer
        .end()
        .map_err(|_trailing| GuardrailParamsError::TrailingData)?;
    Ok(request)
}

pub fn decode_apply_request(body: &[u8]) -> Result<ApplyGuardrailRequest, GuardrailParamsError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let request = serde_path_to_error::deserialize(&mut deserializer)?;
    deserializer
        .end()
        .map_err(|_trailing| GuardrailParamsError::TrailingData)?;
    Ok(request)
}
