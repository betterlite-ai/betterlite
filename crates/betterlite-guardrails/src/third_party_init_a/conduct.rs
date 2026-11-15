use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::{InitResult, decode};

mod tests;

const DEFAULT_TIMEOUT_SECONDS: f64 = 8.0;
const DEFAULT_TOOL_NAME: &str = "llm_call";

#[derive(Debug, Clone)]
pub struct ConductArgs {
    pub name: String,
    pub api_url: Option<String>,
    pub agent_token: Option<SecretString>,
    pub workspace_id: Option<String>,
    pub tool_name: String,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: f64,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

pub fn initialize_conduct(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<ConductArgs> {
    let fields = &params.base.fields;
    let extra = &params.base.extra.0;
    Ok(ConductArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        api_url: fields.api_base.clone(),
        agent_token: fields.api_key.clone(),
        workspace_id: decode("workspace_id", extra.get("workspace_id"))?,
        tool_name: decode("tool_name", extra.get("tool_name"))?
            .unwrap_or_else(|| DEFAULT_TOOL_NAME.to_owned()),
        unreachable_fallback: fields.unreachable_fallback,
        timeout: fields.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
    })
}
