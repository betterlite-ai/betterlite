use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::error::ThirdPartyInitBError;
use super::{InitResult, ParamsView, decode, defaulted};

mod tests;

const DEFAULT_ACTION: &str = "monitor";

#[derive(Debug, Clone)]
pub struct PillarArgs {
    pub name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub on_flagged_action: Option<String>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub async_mode: Option<bool>,
    pub persist_session: Option<bool>,
    pub include_scanners: Option<bool>,
    pub include_evidence: Option<bool>,
    pub fallback_on_error: Option<String>,
    pub timeout: Option<f64>,
}

pub fn initialize_pillar(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<PillarArgs> {
    let name = guardrail_name
        .filter(|name| !name.is_empty())
        .ok_or(ThirdPartyInitBError::MissingPillarName)?;
    let view = ParamsView::parse(&params.base.extra)?;
    Ok(PillarArgs {
        name: name.to_owned(),
        api_key: params.base.fields.api_key.clone(),
        api_base: params.base.fields.api_base.clone(),
        on_flagged_action: defaulted(&view, "on_flagged_action", DEFAULT_ACTION.to_owned())?,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        async_mode: decode("async_mode", view.merged.get("async_mode"))?,
        persist_session: decode("persist_session", view.merged.get("persist_session"))?,
        include_scanners: defaulted(&view, "include_scanners", true)?,
        include_evidence: defaulted(&view, "include_evidence", true)?,
        fallback_on_error: decode("fallback_on_error", view.merged.get("fallback_on_error"))?,
        timeout: decode("timeout", view.merged.get("timeout"))?.or(params.base.fields.timeout),
    })
}
