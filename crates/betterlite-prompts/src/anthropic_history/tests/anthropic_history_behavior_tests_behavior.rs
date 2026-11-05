#![cfg(test)]

use crate::arguments::json::Json;

use super::super::{HistoryError, groups, text, thinking, tools};

macro_rules! json {
    ($($part:tt)*) => {
        serde_json::from_str::<Json>(stringify!($($part)*)).unwrap()
    };
}

fn field<'message>(message: &'message Json, name: &str) -> &'message Json {
    message.as_object().unwrap().get(name).unwrap()
}

#[rstest::rstest]
#[case::empty_string(r#"{"role":"user","content":""}"#)]
#[case::whitespace(r#"{"role":"assistant","content":" \n"}"#)]
fn anthropic_history_behavior_tests_behavior_empty_text(#[case] input_json: &str) {
    let parsed_input: Json = serde_json::from_str(input_json).unwrap();
    assert_eq!(
        field(&text::sanitize_empty_text_content(&parsed_input), "content"),
        &Json::Text("[System: Empty message content sanitised to satisfy protocol]".to_owned())
    );
}

#[test]
fn anthropic_history_behavior_tests_behavior_non_text_role_is_preserved() {
    let message = json!({"role":"system","content":""});
    assert_eq!(text::sanitize_empty_text_content(&message), message);
}

#[test]
fn anthropic_history_behavior_tests_behavior_disabled_policy_returns_original_values() {
    let messages = vec![json!({"role":"user","content":"  "})];
    assert_eq!(
        tools::sanitize_messages(&messages, false).unwrap(),
        messages
    );
}

#[test]
fn anthropic_history_behavior_tests_behavior_text_blocks_change_without_mutating_source() {
    let source =
        json!({"role":"user","content":[{"type":"text","text":" "},{"type":"image","data":"x"}]});
    let sanitized = text::sanitize_empty_text_content(&source);
    let Json::List(sanitized_blocks) = field(&sanitized, "content") else {
        panic!()
    };
    assert_eq!(
        field(sanitized_blocks.first().unwrap(), "text"),
        &Json::Text("[System: Empty message content sanitised to satisfy protocol]".to_owned())
    );
    let Json::List(source_blocks) = field(&source, "content") else {
        panic!()
    };
    assert_eq!(sanitized_blocks.get(1), source_blocks.get(1));
    assert_eq!(
        field(source_blocks.first().unwrap(), "text"),
        &Json::Text(" ".to_owned())
    );
}

#[test]
fn anthropic_history_behavior_tests_behavior_declared_ids_are_string_ids() {
    let message = json!({"tool_calls":[{"id":"a"},{"id":""},{"id":4},{}]});
    assert_eq!(groups::declared_tool_call_ids(&message), ["a"]);
}

#[test]
fn anthropic_history_behavior_tests_behavior_groups_only_matching_contiguous_results() {
    let messages = vec![
        json!({"role":"assistant","tool_calls":[{"id":"a"}]}),
        json!({"role":"tool","tool_call_id":"a"}),
        json!({"role":"tool","tool_call_id":"b"}),
        json!({"role":"user"}),
    ];
    assert_eq!(
        groups::group_tool_exchanges(&messages),
        [vec![0, 1], vec![2], vec![3]]
    );
    assert_eq!(
        groups::iter_tool_exchange_groups(&messages).collect::<Vec<_>>(),
        [vec![0, 1], vec![2], vec![3]]
    );
    assert_eq!(
        groups::repair_indices(&messages, true),
        [vec![0, 1], vec![2], vec![3]]
    );
    assert_eq!(
        groups::repair_indices(&messages, false),
        [vec![0], vec![1], vec![2], vec![3]]
    );
}

#[test]
fn anthropic_history_behavior_tests_behavior_missing_tool_result_gets_named_placeholder() {
    let messages = vec![
        json!({"role":"assistant","tool_calls":[{"id":"x","function":{"name":"read"}},{"id":"y","function":{"name":"search"}}]}),
        json!({"role":"tool","tool_call_id":"x","content":"existing"}),
    ];
    let output = tools::sanitize_messages(&messages, true).unwrap();
    assert_eq!(output.len(), 3);
    let [_, existing, missing] = output.as_slice() else {
        panic!()
    };
    assert_eq!(field(existing, "tool_call_id"), &Json::Text("x".to_owned()));
    assert_eq!(
        field(existing, "content"),
        &Json::Text("existing".to_owned())
    );
    assert_eq!(field(missing, "tool_call_id"), &Json::Text("y".to_owned()));
    let Json::Text(content) = field(missing, "content") else {
        panic!()
    };
    assert!(content.contains("search"));
    let (repair, consumed) =
        tools::add_missing_tool_results(messages.first().unwrap(), &messages, 0);
    assert_eq!(repair, output);
    assert_eq!(consumed, 1);
}

#[test]
fn anthropic_history_behavior_tests_behavior_orphan_tool_result_is_dropped() {
    let messages = vec![json!({"role":"tool","tool_call_id":"missing","content":"secret"})];
    assert!(
        tools::sanitize_messages(&messages, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn anthropic_history_behavior_tests_behavior_result_must_match_latest_assistant() {
    let previous = vec![
        json!({"role":"assistant","tool_calls":[{"id":"old"}]}),
        json!({"role":"assistant","content":"new turn"}),
    ];
    let result = json!({"role":"tool","tool_call_id":"old"});
    assert!(tools::is_orphaned_tool_result(&result, &previous));
}

#[test]
fn anthropic_history_behavior_tests_behavior_matching_tool_result_is_not_orphaned() {
    let previous = vec![json!({"role":"assistant","tool_calls":[{"id":"call"}]})];
    let result = json!({"role":"tool","tool_call_id":"call"});
    assert!(!tools::is_orphaned_tool_result(&result, &previous));
}

#[test]
fn anthropic_history_behavior_tests_behavior_disabled_policy_keeps_orphan_tool_result() {
    let messages = vec![json!({"role":"tool","tool_call_id":"missing"})];
    assert_eq!(
        tools::sanitize_messages(&messages, false).unwrap(),
        messages
    );
}

#[test]
fn anthropic_history_behavior_tests_behavior_invalid_message_has_indexed_error() {
    assert!(matches!(
        tools::sanitize_messages(&[json!(null)], true),
        Err(HistoryError::MessageNotObject { index: 0 })
    ));
}

#[test]
fn anthropic_history_behavior_tests_behavior_duplicate_result_keeps_last() {
    let messages = vec![
        json!({"role":"assistant","tool_calls":[{"id":"x"}]}),
        json!({"role":"tool","tool_call_id":"x","content":"first"}),
        json!({"role":"tool","tool_call_id":"x","content":"last"}),
    ];
    let output = tools::sanitize_messages(&messages, true).unwrap();
    assert_eq!(output.len(), 2);
    let [_, retained] = output.as_slice() else {
        panic!()
    };
    assert_eq!(field(retained, "content"), &Json::Text("last".to_owned()));
}

#[test]
fn anthropic_history_behavior_tests_behavior_skips_only_reinserted_results() {
    let messages = vec![
        json!({"role":"assistant","tool_calls":[{"id":"present"},{"id":"missing"}]}),
        json!({"role":"user","content":"keep this message"}),
        json!({"role":"tool","tool_call_id":"present","content":"existing result"}),
    ];
    let output = tools::sanitize_messages(&messages, true).unwrap();
    assert_eq!(output.len(), 4);
    let [_, existing, missing, preserved] = output.as_slice() else {
        panic!()
    };
    assert_eq!(
        field(existing, "tool_call_id"),
        &Json::Text("present".to_owned())
    );
    assert_eq!(
        field(missing, "tool_call_id"),
        &Json::Text("missing".to_owned())
    );
    assert_eq!(Some(preserved), messages.get(1));
}

#[test]
fn anthropic_history_behavior_tests_behavior_drops_only_unsignable_thinking_blocks() {
    let blocks = vec![
        json!({"type":"thinking","thinking":"no signature"}),
        json!({"type":"thinking","thinking":"signed","signature":"sig"}),
        json!({"type":"thinking","thinking":" ","signature":"sig"}),
        json!({"type":"thinking","thinking":"encrypted","signature":"litellm_encrypted_reasoning:opaque"}),
        json!({"type":"redacted_thinking","data":"litellm_encrypted_reasoning:opaque"}),
        json!({"type":"redacted_thinking","data":"provider-owned"}),
        json!({"type":"text","text":"keep"}),
    ];
    assert_eq!(
        thinking::drop_unsignable(&blocks),
        [blocks[1].clone(), blocks[5].clone(), blocks[6].clone()]
    );
}
