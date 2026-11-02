use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_encrypted_thinking_blocks_the_anchor();
    assert_signed_thinking_is_the_anchor();
}

// An unsignable (encrypted) thinking part never renders, so the assistant turn carrying only one is skipped when looking for the next rendering turn, and the run cannot anchor.
fn assert_encrypted_thinking_blocks_the_anchor() {
    let encrypted_only = vec![
        text_message("user", "hi"),
        text_message("system", "note"),
        parts_message(
            "assistant",
            vec![thinking_part(
                "secret",
                Some("litellm_encrypted_reasoning:xyz"),
            )],
        ),
        text_message("user", "bye"),
    ];
    let placed = place_mid_conversation_system(&encrypted_only, true);
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
            parts_message(
                "assistant",
                vec![thinking_part(
                    "secret",
                    Some("litellm_encrypted_reasoning:xyz")
                )],
            ),
            text_message("user", "bye"),
        ]
    );
}

// A signed thinking part renders, so the assistant turn carrying it is the first later turn that renders, and the run anchors to the user turn in front of it.
fn assert_signed_thinking_is_the_anchor() {
    let signed = vec![
        text_message("user", "hi"),
        text_message("system", "note"),
        parts_message("assistant", vec![thinking_part("plan", Some("sig123"))]),
        text_message("user", "bye"),
    ];
    let placed = place_mid_conversation_system(&signed, true);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            parts_message("system", vec![text_part("note")]),
            parts_message("assistant", vec![thinking_part("plan", Some("sig123"))]),
            text_message("user", "bye"),
        ]
    );
}
