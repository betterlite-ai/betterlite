use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_non_rendering_neighbour_blocks_the_anchor();
    assert_a_user_turn_before_the_next_assistant_turn_blocks_the_anchor();
}

// The neighbour right after the run renders nothing, so the run cannot anchor there.
fn assert_non_rendering_neighbour_blocks_the_anchor() {
    let non_rendering_neighbour = vec![
        text_message("assistant", "a1"),
        text_message("system", "note"),
        message("user", None),
    ];
    let placed = place_mid_conversation_system(&non_rendering_neighbour, true);
    assert_eq!(
        placed,
        vec![
            text_message("assistant", "a1"),
            parts_message(
                "user",
                vec![
                    text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                    text_part("note")
                ]
            ),
            message("user", None),
        ]
    );
}

// The anchor is found, but the first later turn that renders anything is a user turn, not an assistant turn, so the system message may not sit in front of it.
fn assert_a_user_turn_before_the_next_assistant_turn_blocks_the_anchor() {
    let user_turn_before_assistant_renders = vec![
        text_message("user", "hi"),
        text_message("system", "note"),
        message("assistant", None),
        text_message("user", "bye"),
    ];
    let placed = place_mid_conversation_system(&user_turn_before_assistant_renders, true);
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
            message("assistant", None),
            text_message("user", "bye"),
        ]
    );
}
