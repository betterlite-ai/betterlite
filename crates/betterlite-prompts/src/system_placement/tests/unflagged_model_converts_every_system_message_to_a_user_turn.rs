use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let separate_runs = vec![
        text_message("system", "a"),
        text_message("user", "hi"),
        text_message("system", "b"),
    ];
    let placed_separate = place_mid_conversation_system(&separate_runs, false);
    assert_eq!(
        placed_separate,
        vec![
            parts_message(
                "user",
                vec![text_part(text_parts::CONVERTED_SYSTEM_NOTE), text_part("a")]
            ),
            text_message("user", "hi"),
            parts_message(
                "user",
                vec![text_part(text_parts::CONVERTED_SYSTEM_NOTE), text_part("b")]
            ),
        ]
    );

    // A run of adjacent system messages becomes one converted user turn per message, not one merged turn for the whole run.
    let adjacent_run = vec![
        text_message("system", "a"),
        text_message("system", "b"),
        text_message("user", "hi"),
    ];
    let placed_adjacent = place_mid_conversation_system(&adjacent_run, false);
    assert_eq!(
        placed_adjacent,
        vec![
            parts_message(
                "user",
                vec![text_part(text_parts::CONVERTED_SYSTEM_NOTE), text_part("a")]
            ),
            parts_message(
                "user",
                vec![text_part(text_parts::CONVERTED_SYSTEM_NOTE), text_part("b")]
            ),
            text_message("user", "hi"),
        ]
    );
}
