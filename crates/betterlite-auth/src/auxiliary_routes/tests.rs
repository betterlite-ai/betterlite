#![cfg(test)]

use rstest::rstest;

use super::{
    AGENT_INFERENCE_ROUTES, AGENT_MANAGEMENT_ROUTES, AGENT_ROUTES, APPLY_GUARDRAIL_ROUTES,
    COMPLIANCE_CHECK_ROUTES, GOOGLE_ROUTES, INFO_ROUTES, LITELLM_NATIVE_ROUTES,
    MAPPED_PASS_THROUGH_ROUTES, MCP_INFERENCE_ROUTES, MCP_MANAGEMENT_ROUTES, MCP_ROUTES,
    MODEL_INFO_ROUTES, OPENAI_ROUTE_NAMES, PASSTHROUGH_ROUTES_WILDCARD, SELF_MANAGED_ROUTES,
    UI_ROUTES, public_routes,
};

#[test]
fn openai_names_have_expected_count() {
    assert_eq!(OPENAI_ROUTE_NAMES.len(), 8);
}

#[rstest]
#[case::mcp_sse_messages(MCP_INFERENCE_ROUTES, 3, "/mcp/sse/messages")]
#[case::mcp_sse_messages_trailing_slash(MCP_INFERENCE_ROUTES, 4, "/mcp/sse/messages/")]
#[case::mcp_management_path(MCP_MANAGEMENT_ROUTES, 1, concat!("/v1/mcp/server/", "{path:path}"))]
#[case::agent_card(AGENT_INFERENCE_ROUTES, 5, concat!("/a2a/", "{agent_id}", "/.well-known/agent-card.json"))]
#[case::google_converter(GOOGLE_ROUTES, 0, concat!("/v1beta/models/", "{model_name:path}", ":countTokens"))]
#[case::self_managed_agent(SELF_MANAGED_ROUTES, 64, "/v1/agents")]
fn route_literal_is_pinned(#[case] routes: &[&str], #[case] index: usize, #[case] literal: &str) {
    assert_eq!(routes.get(index), Some(&literal));
}

#[test]
fn every_table_has_expected_length() {
    assert_eq!(OPENAI_ROUTE_NAMES.len(), 8);
    assert_eq!(MAPPED_PASS_THROUGH_ROUTES.len(), 28);
    assert_eq!(PASSTHROUGH_ROUTES_WILDCARD.len(), 28);
    assert_eq!(LITELLM_NATIVE_ROUTES.len(), 19);
    assert_eq!(MCP_INFERENCE_ROUTES.len(), 15);
    assert_eq!(MCP_MANAGEMENT_ROUTES.len(), 3);
    assert_eq!(MCP_ROUTES.len(), 18);
    assert_eq!(AGENT_INFERENCE_ROUTES.len(), 6);
    assert_eq!(AGENT_MANAGEMENT_ROUTES.len(), 5);
    assert_eq!(AGENT_ROUTES.len(), 11);
    assert_eq!(GOOGLE_ROUTES.len(), 15);
}

#[test]
fn remaining_tables_have_expected_length() {
    assert_eq!(APPLY_GUARDRAIL_ROUTES.len(), 1);
    assert_eq!(MODEL_INFO_ROUTES.len(), 3);
    assert_eq!(INFO_ROUTES.len(), 24);
    assert_eq!(UI_ROUTES.len(), 45);
    assert_eq!(COMPLIANCE_CHECK_ROUTES.len(), 2);
    assert_eq!(SELF_MANAGED_ROUTES.len(), 69);
}

#[test]
fn wildcard_suffix_is_preserved() {
    assert!(
        PASSTHROUGH_ROUTES_WILDCARD
            .iter()
            .all(|route| route.ends_with("/*"))
    );
}

#[test]
fn composite_routes_keep_legacy_union_order() {
    let mcp_expected = [MCP_INFERENCE_ROUTES, MCP_MANAGEMENT_ROUTES].concat();
    let agent_expected = [AGENT_INFERENCE_ROUTES, AGENT_MANAGEMENT_ROUTES].concat();
    assert_eq!(MCP_ROUTES, mcp_expected.as_slice());
    assert_eq!(AGENT_ROUTES, agent_expected.as_slice());
}

#[test]
fn wildcard_routes_are_derived_from_mapped_routes() {
    let expected: Vec<String> = MAPPED_PASS_THROUGH_ROUTES
        .iter()
        .map(|route| format!("{route}/*"))
        .collect();
    let actual: Vec<String> = PASSTHROUGH_ROUTES_WILDCARD
        .iter()
        .map(|route| (*route).to_owned())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn ui_routes_end_with_info_routes() {
    let info_tail: Vec<_> = UI_ROUTES
        .iter()
        .skip(UI_ROUTES.len() - INFO_ROUTES.len())
        .copied()
        .collect();
    assert_eq!(info_tail, INFO_ROUTES);
}

#[test]
fn self_managed_routes_end_with_agent_management_routes() {
    let agent_tail: Vec<_> = SELF_MANAGED_ROUTES
        .iter()
        .skip(SELF_MANAGED_ROUTES.len() - AGENT_MANAGEMENT_ROUTES.len())
        .copied()
        .collect();
    assert_eq!(agent_tail, AGENT_MANAGEMENT_ROUTES);
}

#[test]
fn ui_routes_retain_info_duplicates() {
    let model_count = UI_ROUTES
        .iter()
        .filter(|route| **route == "/model/info")
        .count();
    assert_eq!(model_count, 2);
}

#[test]
fn model_info_excludes_v2() {
    assert!(!MODEL_INFO_ROUTES.contains(&"/v2/model/info"));
}

#[test]
fn self_managed_is_not_public() {
    let public = public_routes();
    assert!(
        SELF_MANAGED_ROUTES
            .iter()
            .all(|route| !public.contains(route))
    );
}
