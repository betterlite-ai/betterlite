use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_anchor_depends_on_whether_thinking_renders(
        Some("litellm_encrypted_reasoning:xyz"),
        parts_message(
            "user",
            vec![
                text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                text_part("note"),
            ],
        ),
    );
    assert_anchor_depends_on_whether_thinking_renders(
        Some("sig123"),
        parts_message("system", vec![text_part("note")]),
    );
}

// An unsignable (encrypted) thinking part never renders, so the assistant turn carrying only one is skipped when looking for the next rendering turn and the run converts in place instead of anchoring; a signed one renders, so the run anchors to the user turn in front of the assistant turn that carries it.
fn assert_anchor_depends_on_whether_thinking_renders(
    signature: Option<&str>,
    system_turn: Message,
) {
    let messages = vec![
        text_message("user", "hi"),
        text_message("system", "note"),
        parts_message("assistant", vec![thinking_part("thought", signature)]),
        text_message("user", "bye"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            system_turn,
            parts_message("assistant", vec![thinking_part("thought", signature)]),
            text_message("user", "bye"),
        ]
    );
}
