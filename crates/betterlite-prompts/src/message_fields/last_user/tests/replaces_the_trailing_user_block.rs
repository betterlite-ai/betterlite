use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let run = vec![
        text_message("assistant", "hello"),
        text_message("user", "first"),
        text_message("user", "second"),
    ];
    let updated = set_last_user_message(&run, "replacement");
    assert_eq!(updated.len(), 2);
    assert_eq!(updated[0].role, "assistant");
    assert_eq!(updated[1].role, "user");
    assert_eq!(
        updated[1].content,
        Some(Content::Text("replacement".to_owned()))
    );

    let no_trailing_user = vec![text_message("assistant", "hello")];
    let appended = set_last_user_message(&no_trailing_user, "question");
    assert_eq!(appended.len(), 2);
    assert_eq!(
        appended[1].content,
        Some(Content::Text("question".to_owned()))
    );

    assert_eq!(run[1].content, Some(Content::Text("first".to_owned())));
    assert_eq!(no_trailing_user[0].role, "assistant");
}
