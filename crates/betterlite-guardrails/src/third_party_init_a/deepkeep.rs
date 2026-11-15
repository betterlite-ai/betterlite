use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::{InitResult, decode};

mod tests;

#[derive(Debug, Clone)]
pub struct DeepKeepArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub firewall_id: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
    pub extra_headers: Option<Vec<String>>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

pub fn initialize_deepkeep(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<DeepKeepArgs> {
    let fields = &params.base.fields;
    Ok(DeepKeepArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        firewall_id: decode(
            "deepkeep_firewall_id",
            params.base.extra.0.get("deepkeep_firewall_id"),
        )?,
        unreachable_fallback: fields.unreachable_fallback,
        extra_headers: fields.extra_headers.clone(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        timeout: fields.timeout,
    })
}
