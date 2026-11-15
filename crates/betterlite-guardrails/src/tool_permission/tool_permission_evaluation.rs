use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_initializers::tool_permission::ToolDefaultAction;

use super::ToolPermissionGuardrail;
use super::matching::rule_matches_tool;
use super::rule::{ToolDecision, ToolPermissionRule};

mod argument;
mod paths;
mod patterns;
mod tests;

use argument::{ArgumentError, parse_tool_call_arguments};
use patterns::patterns_match_for_rule;

// The tuple LiteLLM's _get_permission_for_tool_call returns, with the absence of a message kept visible
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallPermission {
    pub is_allowed: bool,
    pub rule_id: Option<String>,
    pub message: Option<String>,
}

enum RuleArguments {
    Pass,
    Skipped,
    Failed(String),
}

struct ArgumentCache<'value> {
    arguments: Option<&'value GuardrailValue>,
    parsed: Option<Result<BTreeMap<String, GuardrailValue>, ArgumentError>>,
}

impl ArgumentCache<'_> {
    fn parsed_arguments(
        &mut self,
        tool_name: Option<&str>,
    ) -> Option<&BTreeMap<String, GuardrailValue>> {
        let arguments = self.arguments;
        self.parsed
            .get_or_insert_with(|| parse_tool_call_arguments(arguments, tool_name))
            .as_ref()
            .ok()
    }
}

// LiteLLM's _get_permission_for_tool_call: the first rule that matches completely decides the call
#[must_use]
pub fn get_permission_for_tool_call(
    guardrail: &ToolPermissionGuardrail,
    tool_name: Option<&str>,
    tool_type: Option<&str>,
    arguments: Option<&GuardrailValue>,
) -> ToolCallPermission {
    if tool_name.is_none() && tool_type.is_none() {
        return ToolCallPermission {
            is_allowed: guardrail.args.default_action == ToolDefaultAction::Allow,
            rule_id: None,
            message: None,
        };
    }
    let tool_identifier = tool_name.or(tool_type).unwrap_or("unknown_tool");
    let mut last_pattern_failure = None;
    let mut cache = ArgumentCache {
        arguments,
        parsed: None,
    };
    for rule in &guardrail.rules {
        let (matches, should_check_params) =
            rule_matches_tool(&guardrail.compiled, rule, tool_name, tool_type);
        if !matches {
            continue;
        }
        match rule_arguments(guardrail, rule, should_check_params, tool_name, &mut cache) {
            RuleArguments::Pass => return rule_decision(guardrail, rule, tool_identifier),
            RuleArguments::Skipped => {}
            RuleArguments::Failed(failure) => last_pattern_failure = Some(failure),
        }
    }
    default_decision(guardrail, tool_identifier, last_pattern_failure)
}

fn rule_arguments(
    guardrail: &ToolPermissionGuardrail,
    rule: &ToolPermissionRule,
    should_check_params: bool,
    tool_name: Option<&str>,
    cache: &mut ArgumentCache<'_>,
) -> RuleArguments {
    let needs_patterns = rule
        .allowed_param_patterns
        .as_ref()
        .is_some_and(|patterns| !patterns.is_empty());
    if !needs_patterns || !should_check_params {
        return RuleArguments::Pass;
    }
    let Some(object) = cache.parsed_arguments(tool_name) else {
        return RuleArguments::Skipped;
    };
    patterns_match_for_rule(&guardrail.compiled, rule, object, tool_name)
        .map_or(RuleArguments::Pass, RuleArguments::Failed)
}

fn rule_decision(
    guardrail: &ToolPermissionGuardrail,
    rule: &ToolPermissionRule,
    tool_identifier: &str,
) -> ToolCallPermission {
    let is_allowed = rule.decision == ToolDecision::Allow;
    let verdict = if is_allowed { "allowed" } else { "denied" };
    let rule_id = rule.id.as_str();
    let default_message = format!("Tool '{tool_identifier}' {verdict} by rule '{rule_id}'");
    let message = render_violation_message(
        guardrail,
        &default_message,
        &[("tool_name", tool_identifier), ("rule_id", rule_id)],
    );
    ToolCallPermission {
        is_allowed,
        rule_id: Some(rule.id.clone()),
        message: Some(message),
    }
}

fn default_decision(
    guardrail: &ToolPermissionGuardrail,
    tool_identifier: &str,
    last_pattern_failure: Option<String>,
) -> ToolCallPermission {
    let is_allowed = guardrail.args.default_action == ToolDefaultAction::Allow;
    let verdict = if is_allowed { "allowed" } else { "denied" };
    let default_message = match (last_pattern_failure, is_allowed) {
        (Some(failure), false) => failure,
        _ => format!("Tool '{tool_identifier}' {verdict} by default action"),
    };
    let message = render_violation_message(
        guardrail,
        &default_message,
        &[("tool_name", tool_identifier)],
    );
    ToolCallPermission {
        is_allowed,
        rule_id: None,
        message: Some(message),
    }
}

// LiteLLM renders every permission message through CustomGuardrail.render_violation_message
#[must_use]
pub fn render_violation_message(
    guardrail: &ToolPermissionGuardrail,
    default: &str,
    context: &[(&str, &str)],
) -> String {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: Some(guardrail.args.name.clone()),
            supported_event_hooks: None,
            event_hook: None,
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: guardrail.args.violation_message_template.clone(),
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
    .render_violation_message(default, context)
}
