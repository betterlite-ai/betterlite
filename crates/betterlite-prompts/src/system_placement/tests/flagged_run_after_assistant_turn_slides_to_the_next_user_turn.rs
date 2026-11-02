use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![
        text_message("user", "hi"),
        text_message("assistant", "ok"),
        text_message("system", "note"),
        text_message("user", "next"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            text_message("assistant", "ok"),
            text_message("user", "next"),
            parts_message("system", vec![text_part("note")]),
        ]
    );
}
