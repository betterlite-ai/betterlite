use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let messages = vec![
        message("user", vec![text_part("hi")]),
        message(
            "tool",
            vec![part(
                "tool_result",
                Object::from_iter([("value".to_owned(), Json::Text("ok".to_owned()))]),
            )],
        ),
    ];

    assert_eq!(drop_tool_reference_parts(&messages), messages);

    let non_tool_role_with_reference_shaped_part = vec![message(
        "assistant",
        vec![part("tool_reference", Object::new())],
    )];
    assert_eq!(
        drop_tool_reference_parts(&non_tool_role_with_reference_shaped_part),
        non_tool_role_with_reference_shaped_part,
        "a tool_reference part only matters on a role: tool message"
    );
}
