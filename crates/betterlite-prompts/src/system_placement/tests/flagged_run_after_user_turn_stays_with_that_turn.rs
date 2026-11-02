use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![
        text_message("user", "hi"),
        text_message("system", "be nice"),
        text_message("assistant", "ok"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            parts_message("system", vec![text_part("be nice")]),
            text_message("assistant", "ok"),
        ]
    );
}
