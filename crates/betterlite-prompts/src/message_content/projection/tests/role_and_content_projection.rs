use super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    let messages = vec![
        message("assistant", "a"),
        message("system", "s"),
        message("tool", "t"),
        message("function", "f"),
    ];
    assert_eq!(
        to_content_only_messages(&messages),
        vec![
            ContentOnlyMessage {
                role: "assistant".to_owned(),
                content: "a".to_owned()
            },
            ContentOnlyMessage {
                role: "user".to_owned(),
                content: "t".to_owned()
            },
            ContentOnlyMessage {
                role: "user".to_owned(),
                content: "f".to_owned()
            },
        ]
    );
}
