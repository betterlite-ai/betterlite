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

use error::HookInitError;

pub(crate) type ParamMap = BTreeMap<String, GuardrailValue>;

// LiteLLM's model_dump(exclude_none=True): every declared field and every extra that is not null
fn params_dump(
    params: &LitellmParams,
    vendor: &'static str,
) -> Result<OpaqueParams, HookInitError> {
    let encoded = serde_json::to_value(params)
        .map_err(|_source| HookInitError::UnreadableParams { vendor })?;
    let mut dumped = serde_json::from_value::<OpaqueParams>(encoded)
        .map_err(|_source| HookInitError::UnreadableParams { vendor })?;
    dumped
        .0
        .retain(|_key, value| *value != GuardrailValue::Null);
    Ok(dumped)
}

fn mode_value(
    mode: &LitellmParamsMode,
    vendor: &'static str,
) -> Result<GuardrailValue, HookInitError> {
    let encoded =
        serde_json::to_value(mode).map_err(|_source| HookInitError::UnreadableParams { vendor })?;
    serde_json::from_value(encoded).map_err(|_source| HookInitError::UnreadableParams { vendor })
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
    if let Some(api_key) = api_key {
        extras.0.insert(
            "api_key".to_owned(),
            GuardrailValue::Text(api_key.expose_secret().to_owned()),
        );
    }
    if let Some(api_base) = api_base {
        extras.0.insert(
            "api_base".to_owned(),
            GuardrailValue::Text(api_base.to_owned()),
        );
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

// the declared type of each param LiteLLM reads; a wrong variant is an error naming the field
pub(crate) trait ParamKind: Sized {
    fn read(value: &GuardrailValue) -> Option<Self>;
}

impl ParamKind for String {
    fn read(value: &GuardrailValue) -> Option<Self> {
        match value {
            GuardrailValue::Text(text) => Some(text.clone()),
            _ => None,
        }
    }
}

impl ParamKind for GuardrailFlag {
    fn read(value: &GuardrailValue) -> Option<Self> {
        match value {
            GuardrailValue::Boolean(flag) => Some(*flag),
            _ => None,
        }
    }
}

impl ParamKind for f64 {
    fn read(value: &GuardrailValue) -> Option<Self> {
        match value {
            GuardrailValue::SignedInteger(number) => serde_json::Number::from(*number).as_f64(),
            GuardrailValue::UnsignedInteger(number) => serde_json::Number::from(*number).as_f64(),
            GuardrailValue::Decimal(number) => Some(*number),
            _ => None,
        }
    }
}

impl ParamKind for i64 {
    fn read(value: &GuardrailValue) -> Option<Self> {
        serde_json::to_value(value).ok()?.as_i64()
    }
}

impl ParamKind for ParamMap {
    fn read(value: &GuardrailValue) -> Option<Self> {
        match value {
            GuardrailValue::Object(map) => Some(map.clone()),
            _ => None,
        }
    }
}

pub(crate) fn read<T: ParamKind>(
    map: &ParamMap,
    vendor: &'static str,
    key: &str,
) -> Result<Option<T>, HookInitError> {
    match map.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(value) => T::read(value)
            .map(Some)
            .ok_or_else(|| HookInitError::InvalidField {
                vendor,
                field: key.to_owned(),
            }),
    }
}
