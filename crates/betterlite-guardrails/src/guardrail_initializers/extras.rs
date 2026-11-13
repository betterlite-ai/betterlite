use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use serde::de::DeserializeOwned;

use super::error::InitializerError;

pub(crate) type ExtraRead<T> = Result<T, InitializerError>;

// serde's own text can carry the offending value, so only the field name leaves this module
fn decode<T>(name: &str, value: &GuardrailValue) -> ExtraRead<T>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_vec(value)
        .map_err(|_not_json| InitializerError::InvalidParam(name.to_owned()))?;
    serde_json::from_slice(&encoded)
        .map_err(|_wrong_type| InitializerError::InvalidParam(name.to_owned()))
}

// LiteLLM's X | None fields: the default is None, a null stays None and an absent key takes the default
pub(crate) fn read<T>(extra: &OpaqueParams, key: &str, default: Option<T>) -> ExtraRead<Option<T>>
where
    T: DeserializeOwned,
{
    match extra.0.get(key) {
        None => Ok(default),
        Some(GuardrailValue::Null) => Ok(None),
        Some(value) => decode(key, value).map(Some),
    }
}

// LiteLLM's plain bool fields and its getattr reads: an absent key and a null take the default
pub(crate) fn flag(extra: &OpaqueParams, key: &str, default: bool) -> ExtraRead<GuardrailFlag> {
    match extra.0.get(key) {
        None | Some(GuardrailValue::Null) => Ok(default),
        Some(GuardrailValue::Boolean(value)) => Ok(*value),
        Some(_wrong) => Err(InitializerError::InvalidParam(key.to_owned())),
    }
}
