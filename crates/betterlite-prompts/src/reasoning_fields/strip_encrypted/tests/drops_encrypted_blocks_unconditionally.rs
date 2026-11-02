use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let messages = [message(Some(Content::Parts(vec![
        plain_block(),
        encrypted_block(),
    ])))];

    let stripped = strip_encrypted_reasoning(&messages, None);

    let Some(Content::Parts(parts)) = &stripped[0].content else {
        panic!("content must stay a parts list");
    };
    assert_eq!(parts, &[plain_block()]);
}
