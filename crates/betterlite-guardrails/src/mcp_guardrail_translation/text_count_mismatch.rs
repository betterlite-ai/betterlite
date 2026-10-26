#![cfg(test)]
#[test]
fn mismatch_module_is_registered() {
    assert!(
        super::McpGuardrailError::TextCount {
            returned: 0,
            expected: 1
        }
        .status_code()
            == 400
    );
}
