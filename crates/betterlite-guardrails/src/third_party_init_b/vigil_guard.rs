use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::{InitResult, unreachable_fallback_text};

mod tests;

#[derive(Debug, Clone)]
pub struct VigilGuardArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub unreachable_fallback: String,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

pub fn initialize_vigil_guard(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<VigilGuardArgs> {
    Ok(VigilGuardArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: params.base.fields.api_base.clone(),
        api_key: params.base.fields.api_key.clone(),
        unreachable_fallback: unreachable_fallback_text(params.base.fields.unreachable_fallback)
            .to_owned(),
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
    })
}
