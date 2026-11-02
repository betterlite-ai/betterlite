use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_an_assistant_follower_renders_through(media_part("server_tool_use"));
    assert_an_assistant_follower_renders_through(media_part("web_search_tool_result"));
}

// The first later block that renders anything decides whether a system run may anchor; an assistant block carrying only a server_tool_use part or a part whose kind ends in "_tool_result" renders, so the earlier run still anchors behind the user turn in front of it. Shrinking RENDERED_ASSISTANT_PART_TYPES or dropping the "_tool_result" suffix check would make the search skip this block, land on the later user turn instead, and convert the run in place.
fn assert_an_assistant_follower_renders_through(part: ContentPart) {
    let messages = vec![
        text_message("user", "hi"),
        text_message("system", "note"),
        parts_message("assistant", vec![part.clone()]),
        text_message("user", "bye"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            text_message("user", "hi"),
            parts_message("system", vec![text_part("note")]),
            parts_message("assistant", vec![part]),
            text_message("user", "bye"),
        ]
    );
}
