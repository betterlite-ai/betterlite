use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::error::ThirdPartyInitBError;
use super::{InitResult, decode};

mod tests;

const GROUNDING_STRICTNESS: [&str; 2] = ["BALANCED", "STRICT"];

#[derive(Debug, Clone)]
pub struct XecGuardArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub xecguard_model: Option<String>,
    pub policy_names: Option<Vec<String>>,
    pub block_on_error: Option<bool>,
    pub grounding_strictness: Option<String>,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

pub fn initialize_xecguard(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<XecGuardArgs> {
    let values = &params.base.extra.0;
    let grounding_strictness = decode("grounding_strictness", values.get("grounding_strictness"))?;
    if let Some(strictness) = &grounding_strictness {
        if !GROUNDING_STRICTNESS.contains(&strictness.as_str()) {
            return Err(ThirdPartyInitBError::InvalidParam {
                name: "grounding_strictness".to_owned(),
            });
        }
    }
    Ok(XecGuardArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_base: params.base.fields.api_base.clone(),
        api_key: params.base.fields.api_key.clone(),
        xecguard_model: decode("xecguard_model", values.get("xecguard_model"))?,
        policy_names: decode("policy_names", values.get("policy_names"))?,
        block_on_error: decode("block_on_error", values.get("block_on_error"))?,
        grounding_strictness,
        timeout: params.base.fields.timeout,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
    })
}
