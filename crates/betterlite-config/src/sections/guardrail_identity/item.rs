use std::collections::BTreeMap;

use super::enums::{Role, default_roles};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GuardrailValue {
    Null,
    Boolean(bool),
    SignedInteger(i64),
    UnsignedInteger(u64),
    Decimal(f64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

pub type CallbackArgs = BTreeMap<String, BTreeMap<String, GuardrailValue>>;
pub type GuardrailFlag = bool;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
