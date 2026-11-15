use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::{InitResult, decode};

mod tests;

#[derive(Debug, Clone)]
pub struct AktoArgs {
    pub name: String,
    pub akto_base_url: Option<String>,
    pub akto_api_key: Option<SecretString>,
    pub akto_account_id: Option<String>,
    pub akto_vxlan_id: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
    pub guardrail_timeout: Option<i64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

pub fn initialize_akto(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<AktoArgs> {
    let extra = &params.base.extra.0;
    Ok(AktoArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        akto_base_url: decode("akto_base_url", extra.get("akto_base_url"))?,
        akto_api_key: decode("akto_api_key", extra.get("akto_api_key"))?,
        akto_account_id: decode("akto_account_id", extra.get("akto_account_id"))?,
        akto_vxlan_id: decode("akto_vxlan_id", extra.get("akto_vxlan_id"))?,
        unreachable_fallback: params.base.fields.unreachable_fallback,
        guardrail_timeout: decode("guardrail_timeout", extra.get("guardrail_timeout"))?,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
    })
}
