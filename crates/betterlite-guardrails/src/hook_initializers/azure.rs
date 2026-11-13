use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::HookInitError;
use super::{params_extras, required_name};

mod tests;

const VENDOR: &str = "Azure Content Safety";

#[derive(Debug, Clone)]
pub struct AzureCommon {
    pub guardrail_name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub default_on: GuardrailFlag,
    pub event_hook: LitellmParamsMode,
    pub extras: OpaqueParams,
}

#[derive(Debug, Clone)]
pub enum AzureArgs {
    PromptShield(AzureCommon),
    TextModerations(AzureCommon),
}

pub fn initialize_azure(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<AzureArgs, HookInitError> {
    let (api_key, api_base) = connection(params)?;
    let guardrail_name = required_name(guardrail_name, VENDOR)?;
    let part = match params.guardrail.split('/').nth(1) {
        Some(part) => part,
        None => params.guardrail.as_str(),
    };
    let default_on = params.base.fields.default_on.unwrap_or_default();
    let extras = params_extras(
        params,
        VENDOR,
        Some(&api_key),
        Some(api_base.as_str()),
        default_on,
    )?;
    let common = AzureCommon {
        guardrail_name,
        api_key,
        api_base,
        default_on,
        event_hook: params.mode.clone(),
        extras,
    };
    match part {
        "prompt_shield" => Ok(AzureArgs::PromptShield(common)),
        "text_moderations" => Ok(AzureArgs::TextModerations(common)),
        _ => Err(HookInitError::AzureGuardrailNotValid {
            part: part.to_owned(),
        }),
    }
}

fn connection(params: &LitellmParams) -> Result<(SecretString, String), HookInitError> {
    let api_key = params
        .base
        .fields
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .ok_or(HookInitError::Required {
            vendor: VENDOR,
            setting: "api_key",
        })?;
    let api_base = params
        .base
        .fields
        .api_base
        .clone()
        .filter(|base| !base.is_empty())
        .ok_or(HookInitError::Required {
            vendor: VENDOR,
            setting: "api_base",
        })?;
    Ok((api_key, api_base))
}
