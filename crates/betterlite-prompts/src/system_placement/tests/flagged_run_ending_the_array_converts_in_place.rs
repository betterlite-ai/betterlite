use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![
        text_message("user", "hi"),
        text_message("assistant", "ok"),
        text_message("system", "note"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            text_message("assistant", "ok"),
            parts_message(
                "user",
                vec![
                    text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                    text_part("note")
                ]
            ),
        ]
    );
}
