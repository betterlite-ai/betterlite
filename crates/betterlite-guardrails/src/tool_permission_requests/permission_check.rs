use crate::guardrail_initializers::tool_permission::ToolDefaultAction;
use crate::tool_permission::ToolPermissionGuardrail;
use crate::tool_permission::matching::rule_matches_tool;
use crate::tool_permission::rule::ToolDecision;
use crate::tool_permission::tool_permission_evaluation::render_violation_message;

// Every tool on the request path is checked as a function tool
const REQUEST_TOOL_TYPE: &str = "function";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolVerdict {
    pub is_allowed: bool,
    pub rule_id: Option<String>,
    pub message: String,
}

// LiteLLM's _check_tool_permission: the first rule that matches the name and type decides, and no argument pattern is read
#[must_use]
pub fn check_tool_permission(guardrail: &ToolPermissionGuardrail, tool_name: &str) -> ToolVerdict {
    for rule in &guardrail.rules {
        let (matches, _should_check_params) = rule_matches_tool(
            &guardrail.compiled,
            rule,
            Some(tool_name),
            Some(REQUEST_TOOL_TYPE),
        );
        if !matches {
            continue;
        }
        let is_allowed = rule.decision == ToolDecision::Allow;
        let verdict = if is_allowed { "allowed" } else { "denied" };
        let default_message = format!("Tool '{tool_name}' {verdict} by rule '{}'", rule.id);
        let message = render_violation_message(
            guardrail,
            &default_message,
            &[("tool_name", tool_name), ("rule_id", rule.id.as_str())],
        );
        tracing::debug!("{message}");
        return ToolVerdict {
            is_allowed,
            rule_id: Some(rule.id.clone()),
            message,
        };
    }
    default_verdict(guardrail, tool_name)
}

fn default_verdict(guardrail: &ToolPermissionGuardrail, tool_name: &str) -> ToolVerdict {
    let is_allowed = guardrail.args.default_action == ToolDefaultAction::Allow;
    let verdict = if is_allowed { "allowed" } else { "denied" };
    let default_message = format!("Tool '{tool_name}' {verdict} by default action");
    // LiteLLM passes rule_id None, which a template prints as None
    let message = render_violation_message(
        guardrail,
        &default_message,
        &[("tool_name", tool_name), ("rule_id", "None")],
    );
    tracing::debug!("{message}");
    ToolVerdict {
        is_allowed,
        rule_id: None,
        message,
    }
}

mod tests;
