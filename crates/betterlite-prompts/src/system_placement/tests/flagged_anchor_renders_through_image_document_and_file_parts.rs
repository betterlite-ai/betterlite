use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_a_user_neighbour_renders_through(media_part("image_url"));
    assert_a_user_neighbour_renders_through(media_part("document"));
    assert_a_user_neighbour_renders_through(media_part("file"));
}

// A user message carrying only a non-text rendered part type still renders, so a system run can anchor behind it; a mutation shrinking RENDERED_PART_TYPES would make the run convert in place instead.
fn assert_a_user_neighbour_renders_through(part: ContentPart) {
    let messages = vec![
        parts_message("user", vec![part.clone()]),
        text_message("system", "note"),
        text_message("assistant", "ok"),
    ];
    let placed = place_mid_conversation_system(&messages, true);
    assert_eq!(
        placed,
        vec![
            parts_message("user", vec![part]),
            parts_message("system", vec![text_part("note")]),
            text_message("assistant", "ok"),
        ]
    );
}
