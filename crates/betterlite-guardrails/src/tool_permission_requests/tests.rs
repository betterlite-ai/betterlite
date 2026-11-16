#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;

use super::pre_call_hook;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::tool_permission::{
    ToolDefaultAction, ToolDisallowedAction, ToolPermissionArgs,
};
use crate::tool_permission::ToolPermissionGuardrail;

pub(crate) type Data = BTreeMap<String, GuardrailValue>;

pub(crate) const NAME: &str = "tool-permissions";

// What the hook adds to a request that reaches the model
pub(crate) const APPLIED: &str = r#"{"applied_guardrails": ["tool-permissions"]}"#;

pub(crate) const RULES: &str = r#"[
    {"id": "deny-bash", "tool_name": "Bash", "decision": "deny"},
    {"id": "allow-read", "tool_name": "Read", "decision": "allow"},
    {"id": "deny-write", "tool_name": "Write.*", "decision": "deny"}
]"#;

pub(crate) fn args(
    rules: &str,
    default_action: ToolDefaultAction,
    on_disallowed_action: ToolDisallowedAction,
) -> ToolPermissionArgs {
    ToolPermissionArgs {
        name: NAME.to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        rules: Some(serde_json::from_str(rules).unwrap()),
        default_action,
        on_disallowed_action,
        default_on: true,
        violation_message_template: None,
    }
}

pub(crate) fn guardrail_from(args: ToolPermissionArgs) -> ToolPermissionGuardrail {
    ToolPermissionGuardrail::new(args).unwrap()
}

pub(crate) fn guardrail(
    rules: &str,
    default_action: ToolDefaultAction,
    on_disallowed_action: ToolDisallowedAction,
) -> ToolPermissionGuardrail {
    guardrail_from(args(rules, default_action, on_disallowed_action))
}

pub(crate) fn block_guardrail() -> ToolPermissionGuardrail {
    guardrail(RULES, ToolDefaultAction::Deny, ToolDisallowedAction::Block)
}

pub(crate) fn rewrite_guardrail() -> ToolPermissionGuardrail {
    guardrail(
        RULES,
        ToolDefaultAction::Deny,
        ToolDisallowedAction::Rewrite,
    )
}

// A request body from `"key": value` pairs, each value written as JSON text
pub(crate) fn request(fields: &[(&str, &str)]) -> Data {
    let members: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("\"{key}\": {value}"))
        .collect();
    serde_json::from_str(&format!("{{{}}}", members.join(", "))).unwrap()
}

// The JSON list of function tools with these names
pub(crate) fn tools(names: &[&str]) -> String {
    let entries: Vec<String> = names
        .iter()
        .map(|name| format!(r#"{{"type": "function", "function": {{"name": "{name}"}}}}"#))
        .collect();
    format!("[{}]", entries.join(", "))
}

// The JSON list of legacy functions with these names
pub(crate) fn functions(names: &[&str]) -> String {
    let entries: Vec<String> = names
        .iter()
        .map(|name| format!(r#"{{"name": "{name}"}}"#))
        .collect();
    format!("[{}]", entries.join(", "))
}

// The request the hook forwarded, without the metadata bucket the header lands in
pub(crate) fn forwarded(mut body: Data) -> Data {
    body.remove("metadata");
    body
}

pub(crate) fn run(
    guardrail: &ToolPermissionGuardrail,
    data: &mut Data,
) -> Result<(), GuardrailFailure> {
    pre_call_hook(guardrail, data, &BTreeSet::new())
}
