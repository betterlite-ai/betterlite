#![cfg(test)]
use crate::mcp_guardrail_translation::mapping_registration::{
    GUARDRAIL_TRANSLATION_MAPPINGS, McpCallType, McpGuardrailTranslationHandler,
};
use rstest::rstest;
#[rstest]
#[case::call_tool(r#""call_mcp_tool""#, McpCallType::CallMcpTool)]
#[case::list_tools(r#""list_mcp_tools""#, McpCallType::ListMcpTools)]
fn both_mcp_call_types_map_to_the_mcp_handler(#[case] wire: &str, #[case] call_type: McpCallType) {
    assert_eq!(
        serde_json::from_str::<McpCallType>(wire).unwrap(),
        call_type
    );
    assert_eq!(
        GUARDRAIL_TRANSLATION_MAPPINGS
            .iter()
            .filter(|(mapped, _)| *mapped == call_type)
            .map(|(_, handler)| *handler)
            .collect::<Vec<_>>(),
        vec![McpGuardrailTranslationHandler]
    );
}
#[test]
fn other_call_types_are_not_mcp_call_types() {
    let unknown = serde_json::from_str::<McpCallType>(r#""acompletion""#).unwrap_err();
    assert!(unknown.to_string().contains("acompletion"));
    assert_eq!(GUARDRAIL_TRANSLATION_MAPPINGS.len(), 2);
}
