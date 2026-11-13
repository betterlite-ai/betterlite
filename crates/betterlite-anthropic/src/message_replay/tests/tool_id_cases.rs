use betterlite_prompts::arguments::json::Json;

use super::list_fixture;
use crate::message_replay::tool_ids::{normalize_tool_use_id, sanitize_tool_use_ids};

#[rstest::rstest]
#[case::valid_id("ok_id-1", "ok_id-1")]
#[case::thought_suffix("a__thought__b", "a")]
#[case::repeated_suffix("a__thought__b__thought__c", "a")]
#[case::separator_only("__thought__abc", "tool_use_id")]
#[case::dots_and_colons("functions.Bash:0", "functions_Bash_0")]
#[case::space_and_slash("a b/c", "a_b_c")]
#[case::unicode_character("\u{e9}", "_")]
#[case::empty("", "tool_use_id")]
fn normalize_rewrites_ids_to_the_anthropic_pattern(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(normalize_tool_use_id(raw), expected);
}

#[test]
fn sanitize_rewrites_immediate_tool_blocks_only() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"tool_use","id":"functions.Bash:0","extra":1},{"type":"tool_result","tool_use_id":"call.1","content":[{"type":"tool_use","id":"nested.bad"}]},{"type":"text","text":"keep"}]}]"#,
    );
    assert_eq!(
        sanitize_tool_use_ids(&messages),
        list_fixture(
            r#"[{"role":"assistant","content":[{"type":"tool_use","id":"functions_Bash_0","extra":1},{"type":"tool_result","tool_use_id":"call_1","content":[{"type":"tool_use","id":"nested.bad"}]},{"type":"text","text":"keep"}]}]"#
        )
    );
}

#[test]
fn sanitize_keeps_an_unchanged_message_value_and_key_order() {
    let messages = list_fixture(
        r#"[{"role":"assistant","meta":1,"content":[{"type":"tool_use","id":"ok_id-1"}],"tail":2}]"#,
    );
    let sanitized = sanitize_tool_use_ids(&messages);
    assert_eq!(sanitized, messages);
    let fields = sanitized.first().and_then(Json::as_object).unwrap();
    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(keys, ["role", "meta", "content", "tail"]);
}

#[test]
fn sanitize_preserves_a_rewritten_block_key_order() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"tool_use","input":{"a":1},"id":"a.b","name":"n"}]}]"#,
    );
    let sanitized = sanitize_tool_use_ids(&messages);
    let block = sanitized
        .first()
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("content"))
        .and_then(first_block)
        .and_then(Json::as_object)
        .unwrap();
    let keys: Vec<&str> = block.keys().map(String::as_str).collect();
    assert_eq!(keys, ["type", "input", "id", "name"]);
    assert_eq!(block.get("id"), Some(&Json::Text("a_b".to_owned())));
}

fn first_block(content: &Json) -> Option<&Json> {
    match content {
        Json::List(blocks) => blocks.first(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

#[test]
fn sanitize_ignores_non_string_ids_and_other_block_types() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"tool_use","id":5},{"type":"tool_result","tool_use_id":7},{"type":"server_tool_use"},{"type":"text","id":"a.b"}]}]"#,
    );
    assert_eq!(sanitize_tool_use_ids(&messages), messages);
}

#[test]
fn sanitize_passes_non_list_content_and_non_objects() {
    let messages = list_fixture(
        r#"["text",{"role":"user","content":"hi"},{"role":"user"},{"role":"user","content":{"type":"tool_use","id":"a.b"}}]"#,
    );
    assert_eq!(sanitize_tool_use_ids(&messages), messages);
}

#[test]
fn sanitize_rewrites_server_tool_use_ids() {
    let messages =
        list_fixture(r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"x.y"}]}]"#);
    assert_eq!(
        sanitize_tool_use_ids(&messages),
        list_fixture(r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"x_y"}]}]"#)
    );
}

#[test]
fn sanitize_keeps_non_object_blocks_inside_content() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":["opaque",{"type":"tool_use","id":"a.b"}]}]"#,
    );
    assert_eq!(
        sanitize_tool_use_ids(&messages),
        list_fixture(
            r#"[{"role":"assistant","content":["opaque",{"type":"tool_use","id":"a_b"}]}]"#
        )
    );
}
