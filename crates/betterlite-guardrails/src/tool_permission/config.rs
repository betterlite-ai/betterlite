use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};

use super::error::ToolPermissionError;
use super::rule::ToolPermissionRule;

mod tests;

// The UI schema LiteLLM returns from get_config_model; resolve() validates the action strings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPermissionConfigModel {
    #[serde(default)]
    pub optional_params: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    pub rules: Option<Vec<ToolPermissionRule>>,
    #[serde(default = "default_default_action")]
    pub default_action: String,
    #[serde(default = "default_on_disallowed_action")]
    pub on_disallowed_action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPermissionSettings {
    pub default_action: ToolDefaultAction,
    pub on_disallowed_action: ToolDisallowedAction,
}

impl ToolPermissionConfigModel {
    pub fn resolve(&self) -> Result<ToolPermissionSettings, ToolPermissionError> {
        let default_action = parse_action(
            &self.default_action,
            [
                ("allow", ToolDefaultAction::Allow),
                ("deny", ToolDefaultAction::Deny),
            ],
            |value| ToolPermissionError::InvalidDefaultAction { value },
        )?;
        let on_disallowed_action = parse_action(
            &self.on_disallowed_action,
            [
                ("block", ToolDisallowedAction::Block),
                ("rewrite", ToolDisallowedAction::Rewrite),
            ],
            |value| ToolPermissionError::InvalidDisallowedAction { value },
        )?;
        Ok(ToolPermissionSettings {
            default_action,
            on_disallowed_action,
        })
    }
}

impl GuardrailConfigModelIdentity for ToolPermissionConfigModel {
    fn ui_friendly_name() -> &'static str {
        "LiteLLM Tool Permission Guardrail"
    }
}

fn default_default_action() -> String {
    "deny".to_owned()
}

fn default_on_disallowed_action() -> String {
    "block".to_owned()
}

#[must_use]
pub const fn default_action_text(action: ToolDefaultAction) -> &'static str {
    match action {
        ToolDefaultAction::Allow => "allow",
        ToolDefaultAction::Deny => "deny",
    }
}

#[must_use]
pub const fn disallowed_action_text(action: ToolDisallowedAction) -> &'static str {
    match action {
        ToolDisallowedAction::Block => "block",
        ToolDisallowedAction::Rewrite => "rewrite",
    }
}

// LiteLLM lowercases both action fields before its allow/deny and block/rewrite checks
fn parse_action<Action>(
    value: &str,
    choices: [(&str, Action); 2],
    invalid: fn(String) -> ToolPermissionError,
) -> Result<Action, ToolPermissionError>
where
    Action: Copy,
{
    for (name, action) in choices {
        if value.eq_ignore_ascii_case(name) {
            return Ok(action);
        }
    }
    Err(invalid(value.to_owned()))
}
