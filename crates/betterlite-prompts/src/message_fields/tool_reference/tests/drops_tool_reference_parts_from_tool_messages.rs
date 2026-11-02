use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let tool_with_reference_and_text = message(
        "tool",
        vec![text_part("ok"), part("tool_reference", Object::new())],
    );
    let messages = vec![
        message("user", vec![text_part("hi")]),
        tool_with_reference_and_text.clone(),
    ];

    let dropped = drop_tool_reference_parts(&messages);

    assert_eq!(dropped.len(), 2);
    assert_eq!(dropped[0], messages[0]);
    assert_eq!(dropped[1].role, "tool");
    assert_eq!(
        dropped[1].content,
        Some(Content::Parts(vec![text_part("ok")]))
    );
    assert!(
        matches!(
            &tool_with_reference_and_text.content,
            Some(Content::Parts(parts)) if parts.len() == 2
        ),
        "the source message must not be mutated"
    );

    let reference_only_tool = message("tool", vec![part("tool_reference", Object::new())]);
    let reference_only_dropped = drop_tool_reference_parts(&[reference_only_tool]);
    assert_eq!(reference_only_dropped[0].role, "tool");
    assert_eq!(
        reference_only_dropped[0].content,
        Some(Content::Text(String::new())),
        "a reference-only tool message keeps empty text so the preceding tool_call stays answered"
    );
}
