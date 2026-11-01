use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let messages = [
        message(Some(Content::Text("plain text".to_owned()))),
        message(None),
    ];

    let stripped = strip_encrypted_reasoning(&messages, None);

    assert_eq!(stripped, messages);
}
