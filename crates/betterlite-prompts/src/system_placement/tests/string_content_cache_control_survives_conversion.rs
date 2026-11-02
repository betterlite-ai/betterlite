use super::*;
use crate::system_placement::text_parts::system_message_as_user;

#[test]
fn system_placement_behavior_tests_behavior() {
    let mut message = text_message("system", "note");
    message.provider_specific_fields = Object::from_iter([(
        "cache_control".to_owned(),
        ephemeral_cache_control(Some("1h")),
    )]);

    let converted = system_message_as_user(&message);
    assert_eq!(
        converted,
        parts_message(
            "user",
            vec![
                text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                with_cache_control(text_part("note"), ephemeral_cache_control(Some("1h"))),
            ]
        )
    );
}

// The message-level cache_control belongs to the block built from string content, not to the operator note that precedes it; dropping it would silently remove the client's cache breakpoint from a converted system turn.
fn with_cache_control(mut part: ContentPart, cache_control: Json) -> ContentPart {
    part.fields
        .insert("cache_control".to_owned(), cache_control);
    part
}
