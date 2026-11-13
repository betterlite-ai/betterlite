use betterlite_prompts::arguments::json::Json;

use super::{json_fixture, object_fixture};
use crate::message_replay::cache_control::normalize_cache_control;

fn normalize(payload: &str) -> Json {
    Json::Object(normalize_cache_control(&object_fixture(payload)))
}

fn first_block(content: &Json) -> Option<&Json> {
    match content {
        Json::List(blocks) => blocks.first(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

#[test]
fn portable_control_reduces_the_hint_to_its_type() {
    assert_eq!(
        normalize(r#"{"cache_control":{"type":"ephemeral","ttl":"1h","scope":"global"}}"#),
        json_fixture(r#"{"cache_control":{"type":"ephemeral"}}"#)
    );
}

#[test]
fn portable_control_defaults_a_missing_or_non_string_type_to_ephemeral() {
    assert_eq!(
        normalize(r#"{"cache_control":{}}"#),
        json_fixture(r#"{"cache_control":{"type":"ephemeral"}}"#)
    );
    assert_eq!(
        normalize(r#"{"cache_control":{"type":5}}"#),
        json_fixture(r#"{"cache_control":{"type":"ephemeral"}}"#)
    );
    assert_eq!(
        normalize(r#"{"cache_control":{"type":null}}"#),
        json_fixture(r#"{"cache_control":{"type":"ephemeral"}}"#)
    );
}

#[test]
fn portable_control_keeps_an_empty_string_type() {
    assert_eq!(
        normalize(r#"{"cache_control":{"type":""}}"#),
        json_fixture(r#"{"cache_control":{"type":""}}"#)
    );
}

#[rstest::rstest]
#[case::string(r#""drop""#)]
#[case::null("null")]
#[case::list("[]")]
#[case::number("5")]
#[case::true_literal("true")]
fn portable_control_drops_a_non_mapping_hint(#[case] hint: &str) {
    let payload = format!(r#"{{"cache_control":{hint},"model":"m"}}"#);
    assert_eq!(normalize(&payload), json_fixture(r#"{"model":"m"}"#));
}

#[test]
fn portable_control_moves_the_top_level_hint_last() {
    let normalized = normalize_cache_control(&object_fixture(
        r#"{"cache_control":{"type":"a"},"model":"m"}"#,
    ));
    let keys: Vec<&str> = normalized.keys().map(String::as_str).collect();
    assert_eq!(keys, ["model", "cache_control"]);
}

#[test]
fn portable_control_normalizes_system_and_tools_blocks() {
    assert_eq!(
        normalize(
            r#"{"system":[{"type":"text","text":"s","cache_control":{"type":"ephemeral","ttl":"1h"}},"plain"],"tools":[{"name":"t","cache_control":{"ttl":"5m"},"input_schema":{"cache_control":{"type":"keep"}}}]}"#
        ),
        json_fixture(
            r#"{"system":[{"type":"text","text":"s","cache_control":{"type":"ephemeral"}},"plain"],"tools":[{"name":"t","input_schema":{"cache_control":{"type":"keep"}},"cache_control":{"type":"ephemeral"}}]}"#
        )
    );
}

#[test]
fn portable_control_leaves_non_list_system_tools_and_messages() {
    let payload = r#"{"system":"s","tools":{"cache_control":{"type":"keep"}},"messages":{"cache_control":{"type":"keep"}}}"#;
    assert_eq!(normalize(payload), json_fixture(payload));
}

#[test]
fn portable_control_skips_string_message_content_and_keeps_message_level_hints() {
    let payload = r#"{"messages":[{"role":"user","content":"hi","cache_control":{"type":"ephemeral","ttl":"1h"}}]}"#;
    assert_eq!(normalize(payload), json_fixture(payload));
}

#[test]
fn portable_control_recurses_one_tool_result_level() {
    assert_eq!(
        normalize(
            r#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","cache_control":{"type":"ephemeral","ttl":"1h"},"content":[{"type":"tool_result","tool_use_id":"inner","cache_control":{"type":"ephemeral","ttl":"1h"},"content":[{"type":"text","text":"deep","cache_control":{"type":"ephemeral","ttl":"1h"}}]}]}]}]}"#
        ),
        json_fixture(
            r#"{"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":[{"type":"tool_result","tool_use_id":"inner","content":[{"type":"text","text":"deep","cache_control":{"type":"ephemeral","ttl":"1h"}}],"cache_control":{"type":"ephemeral"}}],"cache_control":{"type":"ephemeral"}}]}]}"#
        )
    );
}

#[test]
fn portable_control_leaves_tool_use_input_and_unrelated_keys() {
    assert_eq!(
        normalize(
            r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"a","input":{"cache_control":{"type":"nested"}},"cache_control":{"type":"ephemeral","ttl":"1h"}}]}],"metadata":{"cache_control":{"type":"keep"}},"thinking":{"type":"enabled"}}"#
        ),
        json_fixture(
            r#"{"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"a","input":{"cache_control":{"type":"nested"}},"cache_control":{"type":"ephemeral"}}]}],"metadata":{"cache_control":{"type":"keep"}},"thinking":{"type":"enabled"}}"#
        )
    );
}

#[test]
fn portable_control_does_not_change_the_input_payload() {
    let payload = object_fixture(
        r#"{"cache_control":{"type":"ephemeral","ttl":"1h"},"system":[{"type":"text","text":"s","cache_control":{"type":"ephemeral","ttl":"1h"}}]}"#,
    );
    let original = payload.clone();
    let normalized = normalize_cache_control(&payload);
    assert_eq!(payload, original);
    assert_ne!(Json::Object(normalized), Json::Object(original));
}

#[test]
fn portable_control_keeps_block_key_order_with_the_hint_last() {
    let normalized = normalize_cache_control(&object_fixture(
        r#"{"system":[{"type":"text","text":"s","cache_control":{"type":"a"},"tail":1}]}"#,
    ));
    let block = normalized
        .get("system")
        .and_then(first_block)
        .and_then(Json::as_object)
        .unwrap();
    let keys: Vec<&str> = block.keys().map(String::as_str).collect();
    assert_eq!(keys, ["type", "text", "tail", "cache_control"]);
}

#[test]
fn portable_control_drops_a_non_mapping_nested_hint() {
    assert_eq!(
        normalize(r#"{"system":[{"type":"text","text":"s","cache_control":"drop"}]}"#),
        json_fixture(r#"{"system":[{"type":"text","text":"s"}]}"#)
    );
}

#[test]
fn portable_control_keeps_blocks_without_a_hint() {
    let payload = r#"{"system":[{"type":"text","text":"s","tail":1}],"tools":[{"name":"t"}]}"#;
    assert_eq!(normalize(payload), json_fixture(payload));
}

#[test]
fn portable_control_keeps_message_level_hints_untouched() {
    let payload = r#"{"messages":[{"role":"assistant","content":[],"cache_control":{"type":"ephemeral","ttl":"1h"}}]}"#;
    assert_eq!(normalize(payload), json_fixture(payload));
}
