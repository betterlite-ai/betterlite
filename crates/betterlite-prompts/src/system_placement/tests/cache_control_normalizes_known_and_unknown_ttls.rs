use super::*;
use crate::system_placement::anthropic_system::anthropic_system_messages;

#[test]
fn system_placement_behavior_tests_behavior() {
    let message = parts_message(
        "system",
        vec![
            with_cache_control(text_part("a"), ephemeral_cache_control(Some("1h"))),
            with_cache_control(text_part("b"), ephemeral_cache_control(Some("5m"))),
            with_cache_control(text_part("c"), ephemeral_cache_control(None)),
            with_cache_control(text_part("d"), ephemeral_cache_control(Some("10m"))),
            with_cache_control(
                text_part("e"),
                Json::Object(Object::from_iter([(
                    "type".to_owned(),
                    Json::Text("other".to_owned()),
                )])),
            ),
            text_part("f"),
        ],
    );
    let wire = anthropic_system_messages(&message);
    let blocks = &wire.first().expect("one anthropic system message").content;

    assert_eq!(
        blocks.first().and_then(cache_control_fields),
        Some(ephemeral_cache_control(Some("1h")))
    );
    assert_eq!(
        blocks.get(1).and_then(cache_control_fields),
        Some(ephemeral_cache_control(Some("5m")))
    );
    assert_eq!(
        blocks.get(2).and_then(cache_control_fields),
        Some(ephemeral_cache_control(None))
    );
    assert_eq!(
        blocks.get(3).and_then(cache_control_fields),
        Some(ephemeral_cache_control(None))
    );
    assert_eq!(blocks.get(4).and_then(cache_control_fields), None);
    assert_eq!(blocks.get(5).and_then(cache_control_fields), None);
}

fn with_cache_control(mut part: ContentPart, cache_control: Json) -> ContentPart {
    part.fields
        .insert("cache_control".to_owned(), cache_control);
    part
}

fn cache_control_fields(part: &ContentPart) -> Option<Json> {
    part.fields.get("cache_control").cloned()
}
