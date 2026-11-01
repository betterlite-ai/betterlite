use std::slice;

use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let message = assistant_message(
        "partial",
        Object::from_iter([
            ("prefix".to_owned(), Json::Bool(true)),
            ("cache_control".to_owned(), Json::Bool(true)),
        ]),
    );
    let converted = convert_prefix_messages(slice::from_ref(&message));

    assert_eq!(converted.len(), 2);
    assert_eq!(converted[0].role, "system");
    assert_eq!(
        converted[0].content,
        Some(Content::Text(PREFIX_CONTINUATION_PROMPT.to_owned()))
    );
    assert_eq!(converted[1].role, "assistant");
    assert_eq!(
        converted[1].content,
        Some(Content::Text("partial".to_owned()))
    );
    assert!(!converted[1].provider_specific_fields.contains_key("prefix"));
    assert_eq!(
        converted[1].provider_specific_fields.get("cache_control"),
        Some(&Json::Bool(true))
    );
    assert!(
        message.provider_specific_fields.contains_key("prefix"),
        "the source message must not be mutated"
    );
}
