#![cfg(test)]
#[test]
fn both_operations_are_registered() {
    assert!(super::guardrail_translation_mappings(super::CallType::CallMcpTool).is_some());
    assert!(super::guardrail_translation_mappings(super::CallType::ListMcpTools).is_some());
}
