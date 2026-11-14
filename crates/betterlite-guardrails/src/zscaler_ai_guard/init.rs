use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use thiserror::Error;

mod tests;

#[derive(Debug, Clone)]
pub struct ZscalerArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub policy_id: Option<i64>,
    pub send_user_api_key_alias: Option<bool>,
    pub send_user_api_key_user_id: Option<bool>,
    pub send_user_api_key_team_id: Option<bool>,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

#[derive(Debug, Error)]
pub enum ZscalerInitError {
    #[error("the guardrail parameter {name} has an invalid value")]
    InvalidParam { name: &'static str },
}

// LiteLLM copies ZscalerAIGuardConfigModel's fields, so an absent send flag is Some(false) and null stays None
pub fn initialize_zscaler(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<ZscalerArgs, ZscalerInitError> {
    let extra = &params.base.extra;
    Ok(ZscalerArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: params.base.fields.api_base.clone(),
        api_key: params.base.fields.api_key.clone(),
        policy_id: optional::<i64>(extra, "policy_id", None)?,
        send_user_api_key_alias: optional::<bool>(extra, "send_user_api_key_alias", Some(false))?,
        send_user_api_key_user_id: optional::<bool>(
            extra,
            "send_user_api_key_user_id",
            Some(false),
        )?,
        send_user_api_key_team_id: optional::<bool>(
            extra,
            "send_user_api_key_team_id",
            Some(false),
        )?,
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
    })
}

fn optional<T>(
    extra: &OpaqueParams,
    name: &'static str,
    default: Option<T>,
) -> Result<Option<T>, ZscalerInitError>
where
    T: DeserializeOwned,
{
    match extra.0.get(name) {
        None => Ok(default),
        Some(GuardrailValue::Null) => Ok(None),
        Some(value) => {
            let bytes = serde_json::to_vec(value)
                .map_err(|_not_json| ZscalerInitError::InvalidParam { name })?;
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_wrong_type| ZscalerInitError::InvalidParam { name })
        }
    }
}
