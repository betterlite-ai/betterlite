#![cfg(test)]

use super::*;
use crate::arguments::json::Object;
use crate::system_placement::cache_control::CacheTtl;

fn message_with_text(text: &str) -> Message {
    Message {
        role: "system".to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_empty_string_content_yields_no_text_parts();
    assert_string_content_picks_up_the_message_level_cache_control();
}

// Anthropic rejects empty text blocks, so empty string content must drop out here rather than build a part the API would reject.
fn assert_empty_string_content_yields_no_text_parts() {
    assert!(text_parts(&message_with_text("")).is_empty());
}

// The text part built from string content carries the cache_control on the message itself, not None; a mutation that drops this lookup would silently remove the client's cache breakpoint.
fn assert_string_content_picks_up_the_message_level_cache_control() {
    let mut message = message_with_text("note");
    message.provider_specific_fields = Object::from_iter([(
        "cache_control".to_owned(),
        Json::Object(Object::from_iter([
            ("type".to_owned(), Json::Text("ephemeral".to_owned())),
            ("ttl".to_owned(), Json::Text("1h".to_owned())),
        ])),
    )]);
    let parts = text_parts(&message);
    assert_eq!(
        parts.first().and_then(|part| part.cache_control),
        Some(CacheControl {
            ttl: Some(CacheTtl::OneHour)
        })
    );
}
