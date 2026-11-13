use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::error::InitializerError;
use super::extras;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDefaultAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDisallowedAction {
    Block,
    Rewrite,
}

#[derive(Debug, Clone)]
pub struct ToolPermissionArgs {
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub rules: Option<Vec<BTreeMap<String, GuardrailValue>>>,
    pub default_action: ToolDefaultAction,
    pub on_disallowed_action: ToolDisallowedAction,
    pub default_on: GuardrailFlag,
    pub violation_message_template: Option<String>,
}

pub fn initialize_tool_permission(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<ToolPermissionArgs, InitializerError> {
    let extra = &params.base.extra;
    let default_action = match extras::read::<String>(extra, "default_action", None)?.as_deref() {
        None | Some("deny") => ToolDefaultAction::Deny,
        Some("allow") => ToolDefaultAction::Allow,
        Some(_unknown) => {
            return Err(InitializerError::InvalidParam("default_action".to_owned()));
        }
    };
    let on_disallowed_action =
        match extras::read::<String>(extra, "on_disallowed_action", None)?.as_deref() {
            None | Some("block") => ToolDisallowedAction::Block,
            Some("rewrite") => ToolDisallowedAction::Rewrite,
            Some(_unknown) => {
                return Err(InitializerError::InvalidParam(
                    "on_disallowed_action".to_owned(),
                ));
            }
        };
    Ok(ToolPermissionArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        rules: extras::read::<Vec<BTreeMap<String, GuardrailValue>>>(extra, "rules", None)?
            .filter(|rules| !rules.is_empty()),
        default_action,
        on_disallowed_action,
        default_on: params.base.fields.default_on.unwrap_or_default(),
        violation_message_template: params.base.fields.violation_message_template.clone(),
    })
}
