use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let messages = vec![text_message("user", "hi")];

    let without_merge = add_system_prompt_to_messages(&messages, "be nice", false);
    assert_eq!(without_merge.len(), 2);
    assert_eq!(without_merge[0].role, "system");
    assert_eq!(
        without_merge[0].content,
        Some(Content::Text("be nice".to_owned()))
    );
    assert_eq!(without_merge[1], messages[0]);

    let merge_requested_but_no_system_present =
        add_system_prompt_to_messages(&messages, "be nice", true);
    assert_eq!(merge_requested_but_no_system_present.len(), 2);
    assert_eq!(merge_requested_but_no_system_present[0].role, "system");

    assert_eq!(
        messages[0].role, "user",
        "the source messages must not be mutated"
    );
}
