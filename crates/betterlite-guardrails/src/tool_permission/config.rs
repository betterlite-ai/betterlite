use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};

use super::error::ToolPermissionError;
use super::rule::ToolPermissionRule;

mod tests;

// The UI schema LiteLLM returns from get_config_model; defaults and case folding follow its validators
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPermissionConfigModel {
    #[serde(default)]
    pub optional_params: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    pub rules: Option<Vec<ToolPermissionRule>>,
    #[serde(default)]
    pub default_action: DefaultAction,
    #[serde(default)]
    pub on_disallowed_action: DisallowedAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DefaultAction(pub ToolDefaultAction);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DisallowedAction(pub ToolDisallowedAction);

impl Default for DefaultAction {
    fn default() -> Self {
        Self(ToolDefaultAction::Deny)
    }
}

impl Default for DisallowedAction {
    fn default() -> Self {
        Self(ToolDisallowedAction::Block)
    }
}

impl TryFrom<String> for DefaultAction {
    type Error = ToolPermissionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        parse_default_action(&value).map(Self)
    }
}

impl From<DefaultAction> for String {
    fn from(action: DefaultAction) -> Self {
        default_action_text(action.0).to_owned()
    }
}

impl TryFrom<String> for DisallowedAction {
    type Error = ToolPermissionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        parse_disallowed_action(&value).map(Self)
    }
}

impl From<DisallowedAction> for String {
    fn from(action: DisallowedAction) -> Self {
        disallowed_action_text(action.0).to_owned()
    }
}

impl GuardrailConfigModelIdentity for ToolPermissionConfigModel {
    fn ui_friendly_name() -> &'static str {
        "LiteLLM Tool Permission Guardrail"
    }
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

pub fn parse_default_action(value: &str) -> Result<ToolDefaultAction, ToolPermissionError> {
    if value.eq_ignore_ascii_case("allow") {
        return Ok(ToolDefaultAction::Allow);
    }
    if value.eq_ignore_ascii_case("deny") {
        return Ok(ToolDefaultAction::Deny);
    }
    Err(ToolPermissionError::InvalidDefaultAction {
        value: value.to_owned(),
    })
}

pub fn parse_disallowed_action(value: &str) -> Result<ToolDisallowedAction, ToolPermissionError> {
    if value.eq_ignore_ascii_case("block") {
        return Ok(ToolDisallowedAction::Block);
    }
    if value.eq_ignore_ascii_case("rewrite") {
        return Ok(ToolDisallowedAction::Rewrite);
    }
    Err(ToolPermissionError::InvalidDisallowedAction {
        value: value.to_owned(),
    })
}
