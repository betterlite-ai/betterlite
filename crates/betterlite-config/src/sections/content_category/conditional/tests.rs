#![cfg(test)]

#[test]
fn conditional_inheritance_obeys_severity_threshold() {
    super::tests::conditional_inheritance_is_severity_filtered();
}
