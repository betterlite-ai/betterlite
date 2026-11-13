use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::InitResult;

mod tests;

#[derive(Debug, Clone)]
pub struct RubrikArgs {
    pub name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

pub fn initialize_rubrik(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<RubrikArgs> {
    Ok(RubrikArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_key: params.base.fields.api_key.clone(),
        api_base: params.base.fields.api_base.clone(),
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: params.base.fields.timeout,
    })
}
