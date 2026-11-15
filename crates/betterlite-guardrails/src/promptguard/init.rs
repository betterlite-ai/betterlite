use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use crate::guardrail_initializers::error::InitializerError;

mod tests;

#[derive(Debug, Clone)]
pub struct PromptGuardArgs {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub block_on_error: Option<bool>,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub name: String,
}

pub fn initialize_promptguard(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<PromptGuardArgs, InitializerError> {
    let block_on_error = match params.base.extra.0.get("block_on_error") {
        None | Some(GuardrailValue::Null) => None,
        Some(GuardrailValue::Boolean(value)) => Some(*value),
        Some(_wrong) => {
            return Err(InitializerError::InvalidParam("block_on_error".to_owned()));
        }
    };
    Ok(PromptGuardArgs {
        api_base: params.base.fields.api_base.clone(),
        api_key: params.base.fields.api_key.clone(),
        block_on_error,
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        name: guardrail_name.unwrap_or_default().to_owned(),
    })
}
