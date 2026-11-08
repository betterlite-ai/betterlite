#![cfg(test)]

#[test]
fn helper_functions_dispatch() {
    let input = serde_json::json!({"function": "messages", "messages": []});
    let decoded = serde_json::from_value(input).unwrap();
    let result = <super::Op as crate::helpers::Helper>::run(&decoded).unwrap();
    assert_eq!(
        result,
        serde_json::json!({"result": {"serialized": null, "parts": []}})
    );
}

#[rstest::rstest]
#[case::guardrail_attributes("guardrail-attributes.yaml")]
#[case::legacy_mapper_genai_system("legacy-mapper-genai-system.yaml")]
#[case::llm_call_genai_attributes("llm-call-genai-attributes.yaml")]
#[case::mcp_list_tools_minimal("mcp-list-tools-minimal.yaml")]
#[case::mcp_tool_call_attributes("mcp-tool-call-attributes.yaml")]
#[case::message_multimodal_content("message-multimodal-content.yaml")]
#[case::responses_api_output("responses-api-output.yaml")]
#[case::service_db_attributes("service-db-attributes.yaml")]
#[case::tool_definition_budget("tool-definition-budget.yaml")]
fn helper_case_matches_litellm(#[case] name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/genai-mapping")
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
