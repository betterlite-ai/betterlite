use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    asserts_a_text_run_is_joined_with_a_blank_line();
    asserts_a_lone_system_message_passes_through_unchanged();
    asserts_a_mixed_text_and_parts_run_is_promoted_to_parts();
    asserts_a_contentless_run_keeps_only_its_first_message();
    asserts_a_non_system_run_passes_through_unchanged();
}

fn asserts_a_text_run_is_joined_with_a_blank_line() {
    let user = text_message("user", "hi");
    let messages = [
        text_message("system", "be nice"),
        text_message("system", "be brief"),
        user.clone(),
    ];

    let merged = merge_adjacent_system_messages(&messages);

    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].role, "system");
    assert_eq!(
        merged[0].content,
        Some(Content::Text("be nice\n\nbe brief".to_owned()))
    );
    assert_eq!(merged[1], user);
}

fn asserts_a_lone_system_message_passes_through_unchanged() {
    let single_system = [text_message("system", "alone"), text_message("user", "hi")];
    assert_eq!(
        merge_adjacent_system_messages(&single_system),
        single_system
    );
}

fn asserts_a_mixed_text_and_parts_run_is_promoted_to_parts() {
    let mixed_run = [
        text_message("system", "plain text"),
        message(
            "system",
            Some(Content::Parts(vec![text_part("structured")])),
        ),
    ];

    let mixed_merged = merge_adjacent_system_messages(&mixed_run);

    assert_eq!(mixed_merged.len(), 1);
    assert_eq!(
        mixed_merged[0].content,
        Some(Content::Parts(vec![
            text_part("plain text"),
            text_part("structured")
        ]))
    );
}

fn asserts_a_contentless_run_keeps_only_its_first_message() {
    let contentless_run = [message("system", None), message("system", None)];

    let contentless_merged = merge_adjacent_system_messages(&contentless_run);

    assert_eq!(
        contentless_merged,
        [contentless_run[0].clone()],
        "a run with no message carrying content keeps only the first message, matching LiteLLM"
    );
}

fn asserts_a_non_system_run_passes_through_unchanged() {
    let non_system_run = [text_message("user", "a"), text_message("assistant", "b")];
    assert_eq!(
        merge_adjacent_system_messages(&non_system_run),
        non_system_run
    );
}
