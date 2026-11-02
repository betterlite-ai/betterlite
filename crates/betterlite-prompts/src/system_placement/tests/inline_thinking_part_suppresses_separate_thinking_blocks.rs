use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let assistant = assistant_with_inline_and_separate_thinking();
    let messages = vec![
        text_message("user", "hi"),
        text_message("system", "note"),
        assistant.clone(),
        text_message("user", "bye"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            parts_message(
                "user",
                vec![
                    text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                    text_part("note")
                ]
            ),
            assistant,
            text_message("user", "bye"),
        ]
    );
}

// An inline thinking part that carries no signature never renders, and the signed thinking_blocks entry it suppresses would otherwise render on its own; deleting the early return in separate_thinking_renders would let the assistant message render, anchor the run behind "hi" instead of converting it in place.
fn assistant_with_inline_and_separate_thinking() -> Message {
    let mut message = parts_message("assistant", vec![thinking_part("inline thought", None)]);
    message.provider_specific_fields = Object::from_iter([(
        "thinking_blocks".to_owned(),
        Json::List(vec![Json::Object(Object::from_iter([
            ("type".to_owned(), Json::Text("thinking".to_owned())),
            (
                "thinking".to_owned(),
                Json::Text("separate thought".to_owned()),
            ),
            ("signature".to_owned(), Json::Text("sig123".to_owned())),
        ]))]),
    )]);
    message
}
