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

    let mixed_messages = [message(Some(Content::Parts(vec![
        encrypted_block(),
        encrypted_block_two(),
    ])))];
    let strip_only_first: &dyn Fn(&ContentPart) -> bool = &|block| *block == encrypted_block();

    let mixed_stripped = strip_encrypted_reasoning(&mixed_messages, Some(strip_only_first));

    let Some(Content::Parts(mixed_parts)) = &mixed_stripped[0].content else {
        panic!("content must stay a parts list");
    };
    assert_eq!(
        mixed_parts,
        &[encrypted_block_two()],
        "should_strip returning true must drop only the block it targets"
    );
}
