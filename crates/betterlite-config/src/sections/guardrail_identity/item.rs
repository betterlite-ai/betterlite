use std::collections::BTreeMap;

use super::enums::{Role, default_roles};
use serde::{Deserialize, Serialize};

pub type CallbackArgs = BTreeMap<String, BTreeMap<String, String>>;
pub type GuardrailFlag = bool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailItem {
    pub callbacks: Vec<String>,
    #[serde(default)]
    pub default_on: bool,
    #[serde(default)]
    pub logging_only: Option<bool>,
    pub guardrail_name: String,
    #[serde(default)]
    pub callback_args: CallbackArgs,
    #[serde(default = "default_enabled_roles")]
    pub enabled_roles: Option<Vec<Role>>,
}

#[must_use]
pub fn default_enabled_roles() -> Option<Vec<Role>> {
    let roles = default_roles();
    Some(roles)
}

impl GuardrailItem {
    #[must_use]
    pub fn new(callbacks: Vec<String>, guardrail_name: String) -> Self {
        let mut item = Self {
            callbacks,
            default_on: false,
            logging_only: None,
            guardrail_name,
            callback_args: CallbackArgs::new(),
            enabled_roles: None,
        };
        item.enabled_roles = Some(default_roles());
        item
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardrailBaseInitKwargs {
    pub guardrail_name: Option<String>,
    pub default_on: GuardrailFlag,
    pub mask_request_content: GuardrailFlag,
    pub mask_response_content: GuardrailFlag,
    pub violation_message_template: Option<String>,
    pub end_session_after_n_fails: Option<i32>,
    pub on_violation: Option<String>,
    pub realtime_violation_message: Option<String>,
    pub on_sensitive_data: Option<String>,
    pub sensitive_data_route_to_model: Option<String>,
    pub sticky_session_routing: GuardrailFlag,
    pub run_in_parallel: GuardrailFlag,
    pub only_scan_new_messages: GuardrailFlag,
}
