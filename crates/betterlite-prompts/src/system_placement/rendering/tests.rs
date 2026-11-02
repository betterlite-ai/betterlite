#![cfg(test)]

use super::*;
use crate::arguments::json::Object;

fn assistant_with_thinking_blocks(signature: &str) -> Message {
    let block = Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text("thinking".to_owned())),
        ("thinking".to_owned(), Json::Text("thought".to_owned())),
        ("signature".to_owned(), Json::Text(signature.to_owned())),
    ]));
    Message {
        role: "assistant".to_owned(),
        content: None,
        name: None,
        provider_specific_fields: Object::from_iter([(
            "thinking_blocks".to_owned(),
            Json::List(vec![block]),
        )]),
    }
}

fn assistant_with_field(key: &str, value: Json) -> Message {
    Message {
        role: "assistant".to_owned(),
        content: None,
        name: None,
        provider_specific_fields: Object::from_iter([(key.to_owned(), value)]),
    }
}

fn part_of_kind(kind: &str) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields: Object::new(),
    }
}

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_separate_thinking_block_renders_with_no_other_signal_present();
    assert_a_text_part_renders_without_ending_in_tool_result();
    assert_a_block_outside_the_known_thinking_types_never_renders();
    assert_each_lone_signal_field_renders_on_its_own();
}

// With no inline parts and none of the other four disjuncts true, only separate_thinking_renders keeps the message on the wire; a mutation folding it into the next disjunct with && would drop it.
fn assert_separate_thinking_block_renders_with_no_other_signal_present() {
    assert!(assistant_renders(&assistant_with_thinking_blocks("sig123")));
}

// A plain text part renders through the RENDERED_ASSISTANT_PART_TYPES membership alone, with no "_tool_result" suffix to fall back on.
fn assert_a_text_part_renders_without_ending_in_tool_result() {
    assert!(assistant_part_renders(&part_of_kind("text")));
}

// A block whose type is neither a rendered assistant part type nor a known thinking type never renders, however is_unsignable_thinking_block classifies it.
fn assert_a_block_outside_the_known_thinking_types_never_renders() {
    let message = assistant_with_field(
        "thinking_blocks",
        Json::List(vec![Json::Object(Object::from_iter([(
            "type".to_owned(),
            Json::Text("other".to_owned()),
        )]))]),
    );
    assert!(!assistant_renders(&message));
}

// tool_calls, function_call and compaction_blocks each keep an assistant message on the wire with no rendering part and no separate thinking block, one disjunct at a time.
fn assert_each_lone_signal_field_renders_on_its_own() {
    let tool_calls = assistant_with_field(
        "tool_calls",
        Json::List(vec![Json::Text("call".to_owned())]),
    );
    assert!(assistant_renders(&tool_calls));

    let function_call = assistant_with_field(
        "function_call",
        Json::Object(Object::from_iter([(
            "name".to_owned(),
            Json::Text("fn".to_owned()),
        )])),
    );
    assert!(assistant_renders(&function_call));

    let compaction_blocks = assistant_with_field(
        "provider_specific_fields",
        Json::Object(Object::from_iter([(
            "compaction_blocks".to_owned(),
            Json::List(vec![Json::Text("block".to_owned())]),
        )])),
    );
    assert!(assistant_renders(&compaction_blocks));
}
