use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{json, serialized};
use crate::datadog_llm_obs::messages::to_dd_messages;

fn projected(messages: &str) -> String {
    serialized(&to_dd_messages(Some(&json(messages))))
}

#[test]
fn projects_text_content_and_joins_its_text_parts() {
    assert_eq!(
        projected(
            r#"[{"role": "user", "content": [{"type": "text", "text": "Hello "}, {"type": "text", "text": "there"}]}]"#
        ),
        r#"[{"role":"user","content":"Hello there"}]"#
    );
}

#[test]
fn keeps_a_content_list_that_carries_no_text() {
    assert_eq!(
        projected(
            r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/a.png"}}]}]"#
        ),
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com/a.png"}}]}]"#
    );
}

#[test]
fn reads_null_and_empty_content_as_empty_text() {
    assert_eq!(
        projected(
            r#"[{"role": "assistant", "content": null}, {"role": "user", "content": []}, {"role": "user"}]"#
        ),
        r#"[{"role":"assistant","content":""},{"role":"user","content":""},{"role":"user","content":""}]"#
    );
}

#[test]
fn keeps_reasoning_content_only_when_it_is_present() {
    assert_eq!(
        projected(r#"[{"role": "assistant", "content": "x", "reasoning_content": ""}]"#),
        r#"[{"role":"assistant","content":"x","reasoning_content":""}]"#
    );
    assert_eq!(
        projected(r#"[{"role": "assistant", "content": "x", "reasoning_content": null}]"#),
        r#"[{"role":"assistant","content":"x"}]"#
    );
}

#[test]
fn projects_openai_tool_calls_with_parsed_and_raw_arguments() {
    assert_eq!(
        projected(
            r#"[{"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\": \"Paris\"}"}},
                {"id": "call_2", "function": {"name": "lookup", "arguments": "not json"}},
                {"function": {"name": "unlinked"}}
            ]}]"#
        ),
        concat!(
            r#"[{"role":"assistant","content":"","tool_calls":["#,
            r#"{"name":"get_weather","arguments":{"city":"Paris"},"tool_id":"call_1","type":"function"},"#,
            r#"{"name":"lookup","arguments":"not json","tool_id":"call_2","type":"function"},"#,
            r#"{"name":"unlinked","arguments":"","tool_id":"","type":"function"}]}]"#
        )
    );
}

#[test]
fn links_openai_tool_results_by_tool_call_id() {
    assert_eq!(
        projected(
            r#"[{"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "function": {"name": "get_weather", "arguments": "{}"}}]},
               {"role": "tool", "tool_call_id": "call_1", "content": "{\"temperature\": 21}"}]"#
        ),
        concat!(
            r#"[{"role":"assistant","content":"","tool_calls":[{"name":"get_weather","arguments":{},"tool_id":"call_1","type":"function"}]},"#,
            r#"{"role":"tool","content":"{\"temperature\": 21}","tool_results":[{"name":"get_weather","result":"{\"temperature\": 21}","tool_id":"call_1","type":"function"}]}]"#
        )
    );
}

#[test]
fn links_a_result_to_a_call_that_appears_later() {
    assert_eq!(
        projected(
            r#"[{"role": "tool", "tool_call_id": "call_9", "content": "late"},
               {"role": "assistant", "tool_calls": [{"id": "call_9", "function": {"name": "later", "arguments": "{}"}}]}]"#
        ),
        concat!(
            r#"[{"role":"tool","content":"late","tool_results":[{"name":"later","result":"late","tool_id":"call_9","type":"function"}]},"#,
            r#"{"role":"assistant","content":"","tool_calls":[{"name":"later","arguments":{},"tool_id":"call_9","type":"function"}]}]"#
        )
    );
}

#[test]
fn the_last_name_for_a_duplicate_tool_id_wins() {
    assert_eq!(
        projected(
            r#"[{"role": "assistant", "tool_calls": [{"id": "dup", "function": {"name": "first", "arguments": "{}"}}]},
               {"role": "tool", "tool_call_id": "dup", "content": "r"},
               {"role": "assistant", "tool_calls": [{"id": "dup", "function": {"name": "second", "arguments": "{}"}}]}]"#
        ),
        concat!(
            r#"[{"role":"assistant","content":"","tool_calls":[{"name":"first","arguments":{},"tool_id":"dup","type":"function"}]},"#,
            r#"{"role":"tool","content":"r","tool_results":[{"name":"second","result":"r","tool_id":"dup","type":"function"}]},"#,
            r#"{"role":"assistant","content":"","tool_calls":[{"name":"second","arguments":{},"tool_id":"dup","type":"function"}]}]"#
        )
    );
}

#[test]
fn projects_anthropic_tool_use_blocks_and_tool_results() {
    assert_eq!(
        projected(
            r#"[{"role": "assistant", "content": [
                    {"type": "text", "text": "Let me check."},
                    {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "Paris"}},
                    {"type": "tool_use", "id": "toolu_2", "name": "lookup", "input": null}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "{\"temp\": 21}"},
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": [{"type": "text", "text": "found"}]},
                    {"type": "text", "text": "thanks"}
                ]}]"#
        ),
        concat!(
            r#"[{"role":"assistant","content":"Let me check.","tool_calls":["#,
            r#"{"name":"get_weather","arguments":{"city":"Paris"},"tool_id":"toolu_1","type":"tool_use"},"#,
            r#"{"name":"lookup","arguments":{},"tool_id":"toolu_2","type":"tool_use"}]},"#,
            r#"{"role":"user","content":"thanks","tool_results":["#,
            r#"{"name":"get_weather","result":"{\"temp\": 21}","tool_id":"toolu_1","type":"function"},"#,
            r#"{"name":"lookup","result":"[{\"type\": \"text\", \"text\": \"found\"}]","tool_id":"toolu_2","type":"function"}]}]"#
        )
    );
}

#[test]
fn a_tool_result_object_serializes_with_python_json_spacing() {
    assert_eq!(
        projected(
            r#"[{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t", "content": {"a": 1, "b": [true, null]}}]}]"#
        ),
        concat!(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","#,
            r#""content":{"a":1,"b":[true,null]}}],"tool_results":["#,
            r#"{"name":"","result":"{\"a\": 1, \"b\": [true, null]}","tool_id":"t","type":"function"}]}]"#
        )
    );
}

#[test]
fn a_tool_result_object_loses_nul_bytes_like_safe_dumps() {
    assert_eq!(
        projected(
            r#"[{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t", "content": {"a": "b\u0000c"}}]}]"#
        ),
        concat!(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","#,
            r#""content":{"a":"b\u0000c"}}],"tool_results":["#,
            r#"{"name":"","result":"{\"a\": \"bc\"}","tool_id":"t","type":"function"}]}]"#
        )
    );
}

#[test]
fn a_tool_result_nested_past_the_structure_cap_carries_the_marker() {
    let nested = (0_i32..105_i32).fold(Json::Number(serde_json::Number::from(1_u8)), |inner, _| {
        Json::List(vec![inner])
    });
    let mut block = IndexMap::new();
    block.insert("type".to_owned(), Json::Text("tool_result".to_owned()));
    block.insert("tool_use_id".to_owned(), Json::Text("t".to_owned()));
    block.insert("content".to_owned(), nested);
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert("content".to_owned(), Json::List(vec![Json::Object(block)]));
    let messages = to_dd_messages(Some(&Json::List(vec![Json::Object(message)])));
    assert!(serialized(&messages).contains("MaxDepthExceeded"));
}

#[rstest]
#[case::string(r#"["hello"]"#, r#"[{"input":"hello"}]"#)]
#[case::number("[7]", r#"[{"input":"7"}]"#)]
fn non_object_messages_fall_back_to_input_records(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(projected(source), expected);
}

#[test]
fn an_empty_message_element_projects_the_empty_message() {
    assert_eq!(projected("[[]]"), r#"[{"role":"","content":""}]"#);
}

#[test]
fn a_nested_message_list_projects_its_first_converted_entry() {
    assert_eq!(
        projected(r#"[[{"role": "user", "content": [{"type": "text", "text": "hi"}]}]]"#),
        r#"[{"role":"user","content":"hi"}]"#
    );
}

#[test]
fn a_message_list_with_a_non_object_content_block_falls_back_to_json() {
    assert_eq!(
        projected(r#"[[{"role": "user", "content": [1]}]]"#),
        r#"[{"input":"{\"role\":\"user\",\"content\":[1]}"}]"#
    );
}

#[test]
fn a_null_or_absent_conversation_projects_nothing() {
    assert_eq!(serialized(&to_dd_messages(None)), "[]");
    let nothing = json("null");
    assert_eq!(serialized(&to_dd_messages(Some(&nothing))), "[]");
}

#[test]
fn a_non_list_conversation_converts_to_an_input_record() {
    assert_eq!(
        projected(r#"{"role": "user"}"#),
        r#"[{"input":"{\"role\":\"user\"}"}]"#
    );
}

#[test]
fn a_tool_message_keeps_its_calls_and_its_own_result() {
    assert_eq!(
        projected(
            r#"[{"role": "tool", "tool_call_id": "c", "content": "r", "tool_calls": [{"id": "c", "function": {"name": "n", "arguments": "{}"}}]}]"#
        ),
        concat!(
            r#"[{"role":"tool","content":"r","tool_calls":["#,
            r#"{"name":"n","arguments":{},"tool_id":"c","type":"function"}],"#,
            r#""tool_results":[{"name":"n","result":"r","tool_id":"c","type":"function"}]}]"#
        )
    );
}

#[test]
fn a_tool_message_without_content_results_in_an_empty_string() {
    assert_eq!(
        projected(r#"[{"role": "tool", "tool_call_id": "c"}]"#),
        r#"[{"role":"tool","content":"","tool_results":[{"name":"","result":"","tool_id":"c","type":"function"}]}]"#
    );
}

#[test]
fn an_anthropic_name_wins_a_shared_id_in_either_message_order() {
    assert_eq!(
        projected(
            r#"[{"role": "assistant", "content": [{"type": "tool_use", "id": "early", "name": "anthropic_early", "input": {}}]},
               {"role": "assistant", "content": null, "tool_calls": [{"id": "early", "function": {"name": "openai_late", "arguments": "{}"}}]},
               {"role": "tool", "tool_call_id": "early", "content": "a"},
               {"role": "assistant", "content": null, "tool_calls": [{"id": "late", "function": {"name": "openai_early", "arguments": "{}"}}]},
               {"role": "assistant", "content": [{"type": "tool_use", "id": "late", "name": "anthropic_late", "input": {}}]},
               {"role": "tool", "tool_call_id": "late", "content": "b"}]"#
        ),
        concat!(
            r#"[{"role":"assistant","content":[{"type":"tool_use","id":"early","name":"anthropic_early","input":{}}],"tool_calls":[{"name":"anthropic_early","arguments":{},"tool_id":"early","type":"tool_use"}]},"#,
            r#"{"role":"assistant","content":"","tool_calls":[{"name":"openai_late","arguments":{},"tool_id":"early","type":"function"}]},"#,
            r#"{"role":"tool","content":"a","tool_results":[{"name":"anthropic_early","result":"a","tool_id":"early","type":"function"}]},"#,
            r#"{"role":"assistant","content":"","tool_calls":[{"name":"openai_early","arguments":{},"tool_id":"late","type":"function"}]},"#,
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"late","name":"anthropic_late","input":{}}],"tool_calls":[{"name":"anthropic_late","arguments":{},"tool_id":"late","type":"tool_use"}]},"#,
            r#"{"role":"tool","content":"b","tool_results":[{"name":"anthropic_late","result":"b","tool_id":"late","type":"function"}]}]"#
        )
    );
}
