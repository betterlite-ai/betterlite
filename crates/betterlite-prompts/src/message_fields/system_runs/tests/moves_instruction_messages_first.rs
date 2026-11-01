use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let user = text_message("user", "hi");
    let system = text_message("system", "be nice");
    let developer = text_message("developer", "be precise");
    let assistant = text_message("assistant", "ok");

    let messages = vec![
        user.clone(),
        system.clone(),
        assistant.clone(),
        developer.clone(),
    ];
    let reordered = system_messages_first(&messages);

    assert_eq!(reordered, vec![system, developer, user, assistant]);

    let all_non_instruction = vec![text_message("user", "a"), text_message("assistant", "b")];
    assert_eq!(
        system_messages_first(&all_non_instruction),
        all_non_instruction
    );
}
