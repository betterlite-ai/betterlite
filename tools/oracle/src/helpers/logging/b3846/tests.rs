#![cfg(test)]

#[test]
fn helper_functions_dispatch() {
    let input = serde_json::json!({"function": "infer_span_kind", "call_type": "embedding"});
    let decoded = serde_json::from_value(input).unwrap();
    let result = <super::Op as crate::helpers::Helper>::run(&decoded).unwrap();
    assert_eq!(result, serde_json::json!({"result": {"kind": "EMBEDDING"}}));
}

#[rstest::rstest]
#[case::langfuse_llm_call("langfuse-llm-call-attributes.yaml")]
#[case::langfuse_usage_breakdown("langfuse-usage-breakdown.yaml")]
#[case::langtrace_operation_and_model("langtrace-operation-and-model.yaml")]
#[case::openinference_message_indexing("openinference-message-indexing.yaml")]
#[case::openinference_tool_definitions_per_index("openinference-tool-definitions-per-index.yaml")]
#[case::openinference_message_shedding_least_valuable(
    "openinference-message-shedding-least-valuable.yaml"
)]
#[case::weave_display_name_and_output("weave-display-name-and-output.yaml")]
#[case::mapper_registry_resolve_and_budget("mapper-registry-resolve-and-budget.yaml")]
#[case::langtrace_integration_request_attributes("langtrace-integration-request-attributes.yaml")]
#[case::span_kind_embed("span-kind-embed.yaml")]
#[case::span_kind_llm("span-kind-llm.yaml")]
#[case::span_kind_unknown("span-kind-unknown.yaml")]
#[case::tool_attributes_optional_and_metadata("tool-attributes-optional-and-metadata.yaml")]
fn helper_case_matches_litellm(#[case] name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/vendor-mapping")
        .join(name);
    let cases = crate::case::load_cases(&[path]).unwrap();
    let Some(crate::case::LoadedCase::Helper(case)) = cases.first() else {
        panic!("expected a helper case");
    };
    assert!(matches!(
        crate::helpers::run_case(case, false).unwrap(),
        crate::helpers::HelperRun::Match
    ));
}
