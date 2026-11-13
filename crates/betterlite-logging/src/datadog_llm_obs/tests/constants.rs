use rstest::rstest;

use super::support::json;
use crate::datadog_llm_obs::constants::{
    AUDIT_GUARDRAIL_FIELDS, COST_DIMENSIONS, GUARDRAIL_PROMPT_FIELDS,
    MAX_PARSED_TOOL_ARGUMENT_CHARS, METADATA_PROMPT_FIELDS, ROUTER_DIMENSIONS, ROUTER_SPAN_FIELDS,
    SAFE_REDACTED_MESSAGE_ROLES, safe_message_role,
};

#[rstest]
#[case::agent("agent", "agent")]
#[case::assistant("assistant", "assistant")]
#[case::developer("developer", "developer")]
#[case::function("function", "function")]
#[case::model("model", "model")]
#[case::system("system", "system")]
#[case::tool("tool", "tool")]
#[case::user("user", "user")]
#[case::unknown("bot", "")]
#[case::different_case("Agent", "")]
fn message_roles_are_kept_only_when_recognized(#[case] role: &str, #[case] expected: &str) {
    let value = json(&format!("\"{role}\""));
    assert_eq!(safe_message_role(Some(&value)), expected);
}

#[test]
fn message_roles_drop_values_that_are_not_strings() {
    assert_eq!(safe_message_role(Some(&json("7"))), "");
    assert_eq!(safe_message_role(Some(&json("null"))), "");
    assert_eq!(safe_message_role(None), "");
}

#[test]
fn the_parsed_argument_bound_is_256_kibibytes() {
    assert_eq!(MAX_PARSED_TOOL_ARGUMENT_CHARS, 256 * 1024);
}

#[test]
fn the_dimension_sets_cover_the_router_and_cost_dimensions() {
    assert_eq!(
        ROUTER_DIMENSIONS,
        [
            "router_tier",
            "router_cause",
            "router_escalated",
            "routed_model"
        ]
    );
    assert_eq!(
        COST_DIMENSIONS,
        [
            "team",
            "user",
            "key_alias",
            "model_group",
            "router_tier",
            "router_cause",
            "router_escalated",
            "routed_model",
        ]
    );
}

#[test]
fn the_router_span_fields_map_onto_the_router_dimensions() {
    assert_eq!(
        ROUTER_SPAN_FIELDS,
        [
            ("tier", "router_tier"),
            ("cause", "router_cause"),
            ("score", "router_score"),
            ("escalated", "router_escalated"),
            ("signals", "router_signals"),
            ("routed_model", "routed_model"),
        ]
    );
}

#[test]
fn the_guardrail_sets_split_audit_fields_from_prompt_carriers() {
    let mut audit = AUDIT_GUARDRAIL_FIELDS;
    let mut expected_audit = [
        "guardrail_name",
        "guardrail_provider",
        "guardrail_mode",
        "guardrail_status",
        "start_time",
        "end_time",
        "duration",
        "masked_entity_count",
        "guardrail_id",
        "policy_template",
        "detection_method",
        "confidence_score",
        "patterns_checked",
        "alert_recipients",
        "risk_score",
        "violation_categories",
        "guardrail_action",
        "guardrail_usage",
        "guardrail_cost",
        "guardrail_cost_by_unit",
        "guardrail_cost_in_spend",
    ];
    let mut prompt = GUARDRAIL_PROMPT_FIELDS;
    let mut expected_prompt = [
        "guardrail_request",
        "guardrail_response",
        "match_details",
        "classification",
    ];
    audit.sort_unstable();
    expected_audit.sort_unstable();
    prompt.sort_unstable();
    expected_prompt.sort_unstable();
    assert_eq!(audit, expected_audit);
    assert_eq!(prompt, expected_prompt);
    assert!(audit.iter().all(|field| !prompt.contains(field)));
}

#[test]
fn the_prompt_carrying_metadata_fields_are_the_five_litellm_keeps_off_redacted_spans() {
    assert_eq!(
        METADATA_PROMPT_FIELDS,
        [
            "routing_decision",
            "requester_metadata",
            "prompt_management_metadata",
            "mcp_tool_call_metadata",
            "vector_store_request_metadata",
        ]
    );
}

#[test]
fn the_safe_role_set_holds_the_eight_roles_litellm_declares() {
    assert_eq!(
        SAFE_REDACTED_MESSAGE_ROLES,
        [
            "agent",
            "assistant",
            "developer",
            "function",
            "model",
            "system",
            "tool",
            "user"
        ]
    );
}
