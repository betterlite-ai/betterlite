#![cfg(test)]

#[test]
fn path_assertion_enforces_data_root_boundary() {
    super::tests::path_assertion_requires_root_containment();
}
