#![cfg(test)]

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::load_helper_cases;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use std::path::Path;

#[test]
fn helper_functions_dispatch() {
    let input = serde_json::json!({"function": "messages", "messages": []});
    let decoded = serde_json::from_value(input).unwrap();
    let result = <super::Op as Helper>::run(&decoded).unwrap();
    assert_eq!(
        result,
        serde_json::json!({"result": {"serialized": null, "parts": []}})
    );
}

#[rstest::rstest]
#[case::guardrail_attributes("guardrail-attributes.yaml")]
#[case::legacy_mapper_genai_system("legacy-mapper-genai-system.yaml")]
#[case::llm_call_failure_error_type("llm-call-failure-error-type.yaml")]
#[case::llm_call_failure_without_class("llm-call-failure-without-class.yaml")]
#[case::llm_call_genai_attributes("llm-call-genai-attributes.yaml")]
#[case::llm_call_image_output_type("llm-call-image-output-type.yaml")]
#[case::mcp_list_tools_minimal("mcp-list-tools-minimal.yaml")]
#[case::mcp_tool_call_attributes("mcp-tool-call-attributes.yaml")]
#[case::mcp_tool_call_without_server("mcp-tool-call-without-server.yaml")]
#[case::message_multimodal_content("message-multimodal-content.yaml")]
#[case::responses_api_output("responses-api-output.yaml")]
#[case::service_db_attributes("service-db-attributes.yaml")]
#[case::tool_definition_budget("tool-definition-budget.yaml")]
fn helper_case_matches_litellm(#[case] name: &str) {
    let path = Path::new(ORACLE_DIR)
        .join("cases/helpers/genai-mapping")
        .join(name);
    let cases = load_helper_cases(&[path]).unwrap();
    let Some(case) = cases.first() else {
        panic!("expected a helper case");
    };
    assert!(matches!(
        run_case(&[crate::GROUP], case, false).unwrap(),
        HelperRun::Match
    ));
}
