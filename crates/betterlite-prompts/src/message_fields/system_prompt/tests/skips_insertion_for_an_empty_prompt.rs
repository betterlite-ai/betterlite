use super::*;
use rstest::rstest;

#[rstest]
#[case::empty_system_prompt_is_a_no_op(false)]
#[case::empty_system_prompt_is_a_no_op_even_with_merge(true)]
fn message_fields_behavior_tests_behavior(#[case] merge_with_first_system: bool) {
    let messages = vec![
        text_message("system", "existing"),
        text_message("user", "hi"),
    ];
    let updated = add_system_prompt_to_messages(&messages, "", merge_with_first_system);
    assert_eq!(updated, messages);
}
