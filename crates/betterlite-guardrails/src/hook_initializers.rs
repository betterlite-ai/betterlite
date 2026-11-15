pub mod agent_365;
pub mod azure;
pub mod error;
pub mod ibm;
pub mod model_armor;
pub mod openai;
pub mod purview;
pub mod secret_lookup;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;

use error::HookInitError;

pub(crate) type ParamMap = BTreeMap<String, GuardrailValue>;

// LiteLLM's model_dump(exclude_none=True): every declared field and every extra that is not null
fn params_dump(
    params: &LitellmParams,
    vendor: &'static str,
) -> Result<OpaqueParams, HookInitError> {
    let encoded = serde_json::to_value(params)
        .map_err(|source| HookInitError::UnreadableParams { vendor, source })?;
    let mut dumped = serde_json::from_value::<OpaqueParams>(encoded)
        .map_err(|source| HookInitError::UnreadableParams { vendor, source })?;
    dumped
        .0
        .retain(|_key, value| *value != GuardrailValue::Null);
    Ok(dumped)
}

fn mode_value(
    mode: &LitellmParamsMode,
    vendor: &'static str,
) -> Result<GuardrailValue, HookInitError> {
    let encoded = serde_json::to_value(mode)
        .map_err(|source| HookInitError::UnreadableParams { vendor, source })?;
    serde_json::from_value(encoded)
        .map_err(|source| HookInitError::UnreadableParams { vendor, source })
}

// the kwargs dict LiteLLM spreads: the non-null dump with the explicit fields set over it
pub(crate) fn params_extras(
    params: &LitellmParams,
    vendor: &'static str,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    default_on: GuardrailFlag,
) -> Result<OpaqueParams, HookInitError> {
    let mut extras = params_dump(params, vendor)?;
    if let Some(key) = api_key {
        extras.0.insert(
            "api_key".to_owned(),
            GuardrailValue::Text(key.expose_secret().to_owned()),
        );
    }
    if let Some(base) = api_base {
        extras
            .0
            .insert("api_base".to_owned(), GuardrailValue::Text(base.to_owned()));
    }
    extras
        .0
        .insert("default_on".to_owned(), GuardrailValue::Boolean(default_on));
    extras
        .0
        .insert("event_hook".to_owned(), mode_value(&params.mode, vendor)?);
    Ok(extras)
}

pub(crate) fn required_name(
    guardrail_name: Option<&str>,
    vendor: &'static str,
) -> Result<String, HookInitError> {
    guardrail_name
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or(HookInitError::Required {
            vendor,
            setting: "guardrail_name",
        })
}

pub(crate) fn required_text(
    map: &ParamMap,
    vendor: &'static str,
    setting: &'static str,
) -> Result<String, HookInitError> {
    let Some(value) = read::<String>(map, vendor, setting)? else {
        return Err(HookInitError::Required { vendor, setting });
    };
    if value.is_empty() {
        return Err(HookInitError::Required { vendor, setting });
    }
    Ok(value)
}

// the declared type of the param; serde's own decoder refuses a value of another type
pub(crate) fn read<T>(
    map: &ParamMap,
    vendor: &'static str,
    key: &str,
) -> Result<Option<T>, HookInitError>
where
    T: DeserializeOwned,
{
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    if *value == GuardrailValue::Null {
        return Ok(None);
    }
    let encoded = serde_json::to_value(value)
        .map_err(|source| HookInitError::UnreadableParams { vendor, source })?;
    serde_json::from_value(encoded)
        .map(Some)
        .map_err(|source| HookInitError::InvalidField {
            vendor,
            field: key.to_owned(),
            source,
        })
}
