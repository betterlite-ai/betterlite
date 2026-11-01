use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let messages = [message(Some(Content::Parts(vec![encrypted_block()])))];
    let keep_everything: &dyn Fn(&ContentPart) -> bool = &|_block| false;

    let stripped = strip_encrypted_reasoning(&messages, Some(keep_everything));

    let Some(Content::Parts(parts)) = &stripped[0].content else {
        panic!("content must stay a parts list");
    };
    assert_eq!(
        parts,
        &[encrypted_block()],
        "should_strip returning false must keep the encrypted block"
    );
}
