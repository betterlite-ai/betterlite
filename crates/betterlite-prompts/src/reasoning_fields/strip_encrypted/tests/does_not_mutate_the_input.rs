use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let original = vec![message(Some(Content::Parts(vec![
        plain_block(),
        encrypted_block(),
    ])))];
    let before = original.clone();

    let _stripped = strip_encrypted_reasoning(&original, None);

    assert_eq!(
        original, before,
        "the input messages and their content lists must not be mutated"
    );
}
