use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;

use super::error::ThirdPartyInitBError;
use super::{
    InitResult, OptionalParams, read_config_default, read_config_value, read_extra_default,
};

mod tests;

const DEFAULT_ON_FLAGGED_ACTION: &str = "monitor";

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
    let extra = &params.base.extra;
    let optional = OptionalParams::parse(extra)?;
    Ok(PillarArgs {
        name: name.to_owned(),
        api_key: params.base.fields.api_key.clone(),
        api_base: params.base.fields.api_base.clone(),
        on_flagged_action: read_extra_default(
            extra,
            "on_flagged_action",
            DEFAULT_ON_FLAGGED_ACTION.to_owned(),
        )?,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        async_mode: read_config_value(extra, &optional, "async_mode")?,
        persist_session: read_config_value(extra, &optional, "persist_session")?,
        include_scanners: read_config_default(extra, &optional, "include_scanners", true)?,
        include_evidence: read_config_default(extra, &optional, "include_evidence", true)?,
        fallback_on_error: read_config_value(extra, &optional, "fallback_on_error")?,
        timeout: match optional.read::<f64>("timeout")? {
            Some(timeout) => Some(timeout),
            None => params.base.fields.timeout,
        },
    })
}
