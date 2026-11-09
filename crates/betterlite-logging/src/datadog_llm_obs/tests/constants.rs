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
    assert!(
        ROUTER_DIMENSIONS
            .iter()
            .all(|dimension| COST_DIMENSIONS.contains(dimension))
    );
    assert_eq!(COST_DIMENSIONS.len(), 8);
    assert!(COST_DIMENSIONS.contains(&"team"));
    assert!(COST_DIMENSIONS.contains(&"user"));
    assert!(COST_DIMENSIONS.contains(&"key_alias"));
    assert!(COST_DIMENSIONS.contains(&"model_group"));
}

#[test]
fn the_router_span_fields_map_onto_the_router_dimensions() {
    assert_eq!(ROUTER_SPAN_FIELDS.len(), 6);
    assert!(ROUTER_SPAN_FIELDS.contains(&("tier", "router_tier")));
    assert!(ROUTER_SPAN_FIELDS.contains(&("routed_model", "routed_model")));
}

#[test]
fn the_guardrail_sets_split_audit_fields_from_prompt_carriers() {
    assert_eq!(AUDIT_GUARDRAIL_FIELDS.len(), 21);
    assert!(AUDIT_GUARDRAIL_FIELDS.contains(&"guardrail_name"));
    assert!(AUDIT_GUARDRAIL_FIELDS.contains(&"masked_entity_count"));
    assert_eq!(GUARDRAIL_PROMPT_FIELDS.len(), 4);
    assert!(GUARDRAIL_PROMPT_FIELDS.contains(&"guardrail_request"));
    assert!(GUARDRAIL_PROMPT_FIELDS.contains(&"classification"));
    assert!(
        AUDIT_GUARDRAIL_FIELDS
            .iter()
            .all(|field| !GUARDRAIL_PROMPT_FIELDS.contains(field))
    );
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
