use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;
use thiserror::Error;

const API_BASE_KEY: &str = "singulr_api_base";
const API_KEY_KEY: &str = "singulr_api_key";
const APPLICATION_ID_KEY: &str = "singulr_application_id";
const GUARDRAIL_ID_KEY: &str = "singulr_guardrail_id";
const BLOCK_ON_ERROR_KEY: &str = "block_on_error";

#[derive(Debug, Clone)]
pub struct SingulrArgs {
    pub name: String,
    pub singulr_api_key: Option<SecretString>,
    pub singulr_api_base: Option<String>,
    pub singulr_application_id: Option<String>,
    pub singulr_guardrail_id: Option<String>,
    pub block_on_error: Option<bool>,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

#[derive(Debug, Error)]
pub enum SingulrInitError {
    #[error("the Singulr guardrail parameter {field} must be a string")]
    Text { field: &'static str },
    #[error("the Singulr guardrail parameter {field} must be a boolean")]
    Flag { field: &'static str },
}

// LiteLLM's initialize_guardrail: a missing singulr_ field falls back to the shared api_base and api_key
pub fn initialize_singulr(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<SingulrArgs, SingulrInitError> {
    Ok(SingulrArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        singulr_api_key: extra_text(params, API_KEY_KEY)?
            .map(SecretString::from)
            .or_else(|| params.base.fields.api_key.clone()),
        singulr_api_base: extra_text(params, API_BASE_KEY)?
            .or_else(|| params.base.fields.api_base.clone()),
        singulr_application_id: extra_text(params, APPLICATION_ID_KEY)?,
        singulr_guardrail_id: extra_text(params, GUARDRAIL_ID_KEY)?,
        block_on_error: extra_flag(params, BLOCK_ON_ERROR_KEY)?,
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
    })
}

fn extra_value<'params>(
    params: &'params LitellmParams,
    key: &str,
) -> Option<&'params GuardrailValue> {
    params.base.extra.0.get(key)
}

fn extra_text(
    params: &LitellmParams,
    key: &'static str,
) -> Result<Option<String>, SingulrInitError> {
    let Some(value) = extra_value(params, key) else {
        return Ok(None);
    };
    if let GuardrailValue::Text(text) = value {
        return Ok(Some(text.clone()));
    }
    if matches!(value, GuardrailValue::Null) {
        return Ok(None);
    }
    Err(SingulrInitError::Text { field: key })
}

fn extra_flag(params: &LitellmParams, key: &'static str) -> Result<Option<bool>, SingulrInitError> {
    let Some(value) = extra_value(params, key) else {
        return Ok(None);
    };
    if let GuardrailValue::Boolean(flag) = value {
        return Ok(Some(*flag));
    }
    if matches!(value, GuardrailValue::Null) {
        return Ok(None);
    }
    Err(SingulrInitError::Flag { field: key })
}

mod tests;
