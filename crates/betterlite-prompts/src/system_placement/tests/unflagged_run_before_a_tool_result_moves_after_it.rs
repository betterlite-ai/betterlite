use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![
        text_message("user", "hi"),
        text_message("system", "note"),
        text_message("tool", "result"),
        text_message("user", "next"),
    ];
    let placed = place_mid_conversation_system(&messages, false);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            text_message("tool", "result"),
            parts_message(
                "user",
                vec![
                    text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                    text_part("note")
                ]
            ),
            text_message("user", "next"),
        ]
    );

    // A tool run with no preceding system run, even as the very first run, passes through unchanged.
    let no_leading_system = vec![text_message("tool", "result"), text_message("user", "hi")];
    assert_eq!(
        place_mid_conversation_system(&no_leading_system, false),
        no_leading_system
    );
}
