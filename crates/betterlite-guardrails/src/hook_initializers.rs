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
    match guardrail_name.filter(|name| !name.is_empty()) {
        Some(name) => Ok(name.to_owned()),
        None => Err(HookInitError::Required {
            vendor,
            setting: "guardrail_name",
        }),
    }
}

pub(crate) fn required_text(
    map: &ParamMap,
    vendor: &'static str,
    setting: &'static str,
) -> Result<String, HookInitError> {
    text(map, vendor, setting)?
        .filter(|value| !value.is_empty())
        .ok_or(HookInitError::Required { vendor, setting })
}

pub(crate) fn text(
    map: &ParamMap,
    vendor: &'static str,
    key: &str,
) -> Result<Option<String>, HookInitError> {
    match map.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(value)) => Ok(Some(value.clone())),
        Some(_wrong) => Err(HookInitError::InvalidField {
            vendor,
            field: key.to_owned(),
        }),
    }
}

pub(crate) fn optional_flag(
    map: &ParamMap,
    vendor: &'static str,
    key: &str,
) -> Result<Option<GuardrailFlag>, HookInitError> {
    match map.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Boolean(value)) => Ok(Some(*value)),
        Some(_wrong) => Err(HookInitError::InvalidField {
            vendor,
            field: key.to_owned(),
        }),
    }
}

pub(crate) fn number(
    map: &ParamMap,
    vendor: &'static str,
    key: &str,
) -> Result<Option<f64>, HookInitError> {
    match map.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::SignedInteger(value)) => Ok(serde_json::Number::from(*value).as_f64()),
        Some(GuardrailValue::UnsignedInteger(value)) => {
            Ok(serde_json::Number::from(*value).as_f64())
        }
        Some(GuardrailValue::Decimal(value)) => Ok(Some(*value)),
        Some(_wrong) => Err(HookInitError::InvalidField {
            vendor,
            field: key.to_owned(),
        }),
    }
}

pub(crate) fn integer(
    map: &ParamMap,
    vendor: &'static str,
    key: &str,
) -> Result<Option<i64>, HookInitError> {
    match map.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::SignedInteger(value)) => Ok(Some(*value)),
        Some(GuardrailValue::UnsignedInteger(value)) => {
            i64::try_from(*value)
                .map(Some)
                .map_err(|_overflow| HookInitError::InvalidField {
                    vendor,
                    field: key.to_owned(),
                })
        }
        Some(_wrong) => Err(HookInitError::InvalidField {
            vendor,
            field: key.to_owned(),
        }),
    }
}

pub(crate) fn object<'a>(
    map: &'a ParamMap,
    vendor: &'static str,
    key: &str,
) -> Result<Option<&'a ParamMap>, HookInitError> {
    match map.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(value)) => Ok(Some(value)),
        Some(_wrong) => Err(HookInitError::InvalidField {
            vendor,
            field: key.to_owned(),
        }),
    }
}
