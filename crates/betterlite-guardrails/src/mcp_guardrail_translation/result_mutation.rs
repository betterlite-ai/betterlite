#![cfg(test)]
#[test]
fn mutation_module_is_registered() {
    assert_eq!(super::MAX_STRUCTURED_CONTENT_SCAN_DEPTH, 20);
}
