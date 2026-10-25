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
#[case::openai(OPENAI_ROUTE_NAMES)]
#[case::mapped(MAPPED_PASS_THROUGH_ROUTES)]
#[case::wildcards(PASSTHROUGH_ROUTES_WILDCARD)]
#[case::native(LITELLM_NATIVE_ROUTES)]
#[case::mcp_inference(MCP_INFERENCE_ROUTES)]
#[case::mcp_management(MCP_MANAGEMENT_ROUTES)]
#[case::mcp(MCP_ROUTES)]
#[case::agent_inference(AGENT_INFERENCE_ROUTES)]
#[case::agent_management(AGENT_MANAGEMENT_ROUTES)]
#[case::agents(AGENT_ROUTES)]
#[case::google(GOOGLE_ROUTES)]
#[case::guardrail(APPLY_GUARDRAIL_ROUTES)]
#[case::model_info(MODEL_INFO_ROUTES)]
#[case::info(INFO_ROUTES)]
#[case::ui(UI_ROUTES)]
#[case::compliance(COMPLIANCE_CHECK_ROUTES)]
#[case::self_managed(SELF_MANAGED_ROUTES)]
fn every_table_is_nonempty(#[case] routes: &[&str]) {
    assert!(!routes.is_empty());
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
    assert_eq!(MCP_ROUTES[0], MCP_INFERENCE_ROUTES[0]);
    assert_eq!(
        MCP_ROUTES[MCP_INFERENCE_ROUTES.len()],
        MCP_MANAGEMENT_ROUTES[0]
    );
    assert_eq!(AGENT_ROUTES[0], AGENT_INFERENCE_ROUTES[0]);
    assert_eq!(
        AGENT_ROUTES[AGENT_INFERENCE_ROUTES.len()],
        AGENT_MANAGEMENT_ROUTES[0]
    );
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
