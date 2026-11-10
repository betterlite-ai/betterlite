use betterlite_prompts::arguments::json::Json;

use super::list_fixture;
use crate::message_replay::provider_fields::strip_provider_specific_fields;

fn first_block(content: &Json) -> Option<&Json> {
    match content {
        Json::List(blocks) => blocks.first(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

#[test]
fn provider_fields_are_stripped_from_immediate_blocks_only() {
    let messages = list_fixture(
        r#"[{"role":"assistant","provider_specific_fields":{"message":"kept"},"content":[{"type":"text","text":"x","provider_specific_fields":{"a":1}},{"type":"tool_use","id":"t","input":{"provider_specific_fields":{"nested":true}},"provider_specific_fields":null},"opaque"]}]"#,
    );
    assert_eq!(
        strip_provider_specific_fields(&messages),
        list_fixture(
            r#"[{"role":"assistant","provider_specific_fields":{"message":"kept"},"content":[{"type":"text","text":"x"},{"type":"tool_use","id":"t","input":{"provider_specific_fields":{"nested":true}}},"opaque"]}]"#
        )
    );
}

#[test]
fn provider_fields_pass_through_messages_without_list_content() {
    let messages = list_fixture(
        r#"["text",{"role":"user","content":"hi"},{"role":"user"},{"role":"user","content":{"provider_specific_fields":{"a":1}}}]"#,
    );
    assert_eq!(strip_provider_specific_fields(&messages), messages);
}

#[test]
fn provider_fields_keep_a_rewritten_block_key_order() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"x","provider_specific_fields":{"a":1},"tail":2}]}]"#,
    );
    let stripped = strip_provider_specific_fields(&messages);
    let block = stripped
        .first()
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("content"))
        .and_then(first_block)
        .and_then(Json::as_object)
        .unwrap();
    let keys: Vec<&str> = block.keys().map(String::as_str).collect();
    assert_eq!(keys, ["type", "text", "tail"]);
}
