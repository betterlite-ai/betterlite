use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![
        text_message("assistant", "a1"),
        text_message("system", "x"),
        text_message("user", "u1"),
        text_message("system", "y"),
        text_message("user", "u2"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            text_message("assistant", "a1"),
            text_message("user", "u1"),
            text_message("user", "u2"),
            parts_message("system", vec![text_part("x"), text_part("y")]),
        ]
    );
}
