#![cfg(test)]

use super::*;
use crate::arguments::json::Object;

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
}

// Anthropic rejects empty text blocks, so empty string content must drop out here rather than build a part the API would reject.
fn assert_empty_string_content_yields_no_text_parts() {
    assert!(text_parts(&message_with_text("")).is_empty());
}
