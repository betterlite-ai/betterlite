pub mod aim;
pub mod akto;
pub mod aporia;
pub mod cisco;
pub mod conduct;
pub mod crowdstrike;
pub mod deepkeep;
pub mod error;
pub mod generic_api;
pub mod grayswan;
pub mod guardrails_ai;
pub mod headroom;
pub mod lasso;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use serde::de::DeserializeOwned;

use self::error::ThirdPartyInitAError;

mod tests;

pub(crate) type InitResult<T> = Result<T, ThirdPartyInitAError>;
pub(crate) type ParamMap = BTreeMap<String, GuardrailValue>;

// a null reads as absent, and a value of another type than the field's names the field
pub(crate) fn decode<T>(name: &str, value: Option<&GuardrailValue>) -> InitResult<Option<T>>
where
    T: DeserializeOwned,
{
    let Some(present) = value.filter(|entry| **entry != GuardrailValue::Null) else {
        return Ok(None);
    };
    let encoded =
        serde_json::to_vec(present).map_err(|_not_json| ThirdPartyInitAError::InvalidParam {
            name: name.to_owned(),
        })?;
    serde_json::from_slice(&encoded)
        .map(Some)
        .map_err(|_wrong_type| ThirdPartyInitAError::InvalidParam {
            name: name.to_owned(),
        })
}

pub(crate) fn optional_params(extra: &OpaqueParams) -> InitResult<Option<&ParamMap>> {
    match extra.0.get("optional_params") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(entries)) => Ok(Some(entries)),
        Some(_wrong) => Err(ThirdPartyInitAError::InvalidParam {
            name: "optional_params".to_owned(),
        }),
    }
}

// the optional_params entry unless it is null, else the entry of the top-level map the caller passes
pub(crate) fn config_value<'params>(
    optional: Option<&'params ParamMap>,
    top_level: &'params ParamMap,
    name: &str,
) -> Option<&'params GuardrailValue> {
    optional
        .and_then(|entries| entries.get(name))
        .filter(|value| **value != GuardrailValue::Null)
        .or_else(|| top_level.get(name))
}
