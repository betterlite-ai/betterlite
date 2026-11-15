use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::InitResult;

mod tests;

#[derive(Debug, Clone)]
pub struct AimArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub inspect_embeddings: Option<bool>,
    pub timeout: Option<f64>,
}

pub fn initialize_aim(params: &LitellmParams, guardrail_name: Option<&str>) -> InitResult<AimArgs> {
    let fields = &params.base.fields;
    Ok(AimArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        inspect_embeddings: fields.inspect_embeddings,
        timeout: fields.timeout,
    })
}
