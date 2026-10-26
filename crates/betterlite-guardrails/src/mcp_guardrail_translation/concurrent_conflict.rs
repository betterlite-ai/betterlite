#![cfg(test)]
#[test]
fn conflict_module_is_registered() {
    assert!(matches!(
        super::McpGuardrailError::ConflictingRewrite {
            path: "x".to_owned()
        },
        super::McpGuardrailError::ConflictingRewrite { .. }
    ));
}
