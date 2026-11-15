use betterlite_config::sections::cisco_verdict_contracts::{FallbackAction, InspectionType};
use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::ThirdPartyInitAError;
use super::{InitResult, config_value, decode, optional_params};

mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum CiscoRule {
    Name(String),
    Entry(CiscoRuleEntry),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CiscoRuleEntry {
    pub rule_name: Option<String>,
    pub entity_types: Option<Vec<String>>,
    pub rule_id: Option<String>,
    pub classification: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CiscoArgs {
    pub name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub inspection_type: Option<InspectionType>,
    pub inspect_path: Option<String>,
    pub enabled_rules: Option<Vec<CiscoRule>>,
    pub integration_profile_id: Option<String>,
    pub integration_profile_version: Option<String>,
    pub integration_tenant_id: Option<String>,
    pub integration_type: Option<String>,
    pub on_flagged_action: Option<String>,
    pub fallback_on_error: Option<FallbackAction>,
    pub timeout: Option<f64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

pub fn initialize_cisco(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<CiscoArgs> {
    let name = guardrail_name
        .filter(|name| !name.is_empty())
        .ok_or(ThirdPartyInitAError::MissingCiscoName)?;
    let optional = optional_params(&params.base.extra)?;
    // the helper reads the top-level param only when the config set it, null included
    let sent = params
        .base
        .sent_fields()
        .map_err(|source| ThirdPartyInitAError::Params { source })?;
    let lookup = |key: &str| config_value(optional, &sent, key);
    let fields = &params.base.fields;
    Ok(CiscoArgs {
        name: name.to_owned(),
        api_key: fields.api_key.clone(),
        api_base: fields.api_base.clone(),
        inspection_type: decode("inspection_type", lookup("inspection_type"))?,
        inspect_path: decode("inspect_path", lookup("inspect_path"))?,
        enabled_rules: decode("enabled_rules", lookup("enabled_rules"))?,
        integration_profile_id: decode("integration_profile_id", lookup("integration_profile_id"))?,
        integration_profile_version: decode(
            "integration_profile_version",
            lookup("integration_profile_version"),
        )?,
        integration_tenant_id: decode("integration_tenant_id", lookup("integration_tenant_id"))?,
        integration_type: decode("integration_type", lookup("integration_type"))?,
        on_flagged_action: decode("on_flagged_action", lookup("on_flagged_action"))?,
        fallback_on_error: decode("fallback_on_error", lookup("fallback_on_error"))?,
        timeout: decode("timeout", lookup("timeout"))?,
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
    })
}
