use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use secrecy::SecretString;

use super::error::InitializerError;
use super::extras;

mod tests;

#[derive(Debug, Clone)]
pub struct LakeraV2Args {
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub project_id: Option<String>,
    pub payload: Option<bool>,
    pub breakdown: Option<bool>,
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    pub dev_info: Option<bool>,
    pub on_flagged: Option<LakeraOnFlagged>,
    pub skip_system_message: Option<bool>,
    pub skip_tool_message_in_guardrail: Option<bool>,
    pub advisory_system_message: Option<String>,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

pub fn initialize_lakera_v2(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<LakeraV2Args, InitializerError> {
    let extra = &params.base.extra;
    Ok(LakeraV2Args {
        name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        api_base: params.base.fields.api_base.clone(),
        api_key: params.base.fields.api_key.clone(),
        project_id: extras::read(extra, "project_id", None)?,
        payload: extras::read(extra, "payload", Some(true))?,
        breakdown: extras::read(extra, "breakdown", Some(true))?,
        metadata: extras::read(extra, "metadata", None)?,
        dev_info: extras::read(extra, "dev_info", Some(true))?,
        on_flagged: extras::read(extra, "on_flagged", Some(LakeraOnFlagged::Block))?,
        skip_system_message: params.base.fields.skip_system_message,
        skip_tool_message_in_guardrail: params.base.fields.skip_tool_message_in_guardrail,
        advisory_system_message: extras::read(extra, "advisory_system_message", None)?,
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: params.base.fields.timeout,
    })
}
