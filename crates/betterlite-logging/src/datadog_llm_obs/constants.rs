use betterlite_models::decisions::Json;

pub const MAX_PARSED_TOOL_ARGUMENT_CHARS: usize = 256 * 1024;

pub const SAFE_REDACTED_MESSAGE_ROLES: [&str; 8] = [
    "agent",
    "assistant",
    "developer",
    "function",
    "model",
    "system",
    "tool",
    "user",
];

#[rustfmt::skip]
pub const GUARDRAIL_PROMPT_FIELDS: [&str; 4] = [
    "guardrail_request", "guardrail_response", "match_details", "classification",
];

#[rustfmt::skip]
pub const AUDIT_GUARDRAIL_FIELDS: [&str; 21] = [
    "guardrail_name", "guardrail_provider", "guardrail_mode", "guardrail_status",
    "start_time", "end_time", "duration", "masked_entity_count",
    "guardrail_id", "policy_template", "detection_method", "confidence_score",
    "patterns_checked", "alert_recipients", "risk_score", "violation_categories",
    "guardrail_action", "guardrail_usage", "guardrail_cost", "guardrail_cost_by_unit",
    "guardrail_cost_in_spend",
];

#[rustfmt::skip]
pub const METADATA_PROMPT_FIELDS: [&str; 5] = [
    "routing_decision", "requester_metadata", "prompt_management_metadata",
    "mcp_tool_call_metadata", "vector_store_request_metadata",
];

#[rustfmt::skip]
pub const ROUTER_SPAN_FIELDS: [(&str, &str); 6] = [
    ("tier", "router_tier"), ("cause", "router_cause"), ("score", "router_score"),
    ("escalated", "router_escalated"), ("signals", "router_signals"), ("routed_model", "routed_model"),
];

#[rustfmt::skip]
pub const ROUTER_DIMENSIONS: [&str; 4] = [
    "router_tier", "router_cause", "router_escalated", "routed_model",
];

#[rustfmt::skip]
pub const COST_DIMENSIONS: [&str; 8] = [
    "team", "user", "key_alias", "model_group",
    "router_tier", "router_cause", "router_escalated", "routed_model",
];

#[must_use]
pub fn safe_message_role(role: Option<&Json>) -> String {
    match role {
        Some(Json::Text(name)) if SAFE_REDACTED_MESSAGE_ROLES.contains(&name.as_str()) => {
            name.clone()
        }
        Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => String::new(),
    }
}
