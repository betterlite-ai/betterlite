#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{fallback_query, query_for_target, render_tool_intent};
use crate::generic_guardrail_contracts::GuardrailMessage;

fn messages(source: &str) -> Vec<GuardrailMessage> {
    serde_json::from_str(source).unwrap()
}

fn function(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::name_and_arguments(
    r#"{"name":"search","arguments":"{\"q\":\"cats\"}"}"#,
    "search: {\"q\":\"cats\"}"
)]
#[case::name_and_object_arguments(
    r#"{"name":"search","arguments":{"q":"cats"}}"#,
    "search: {\"q\":\"cats\"}"
)]
#[case::name_only(r#"{"name":"search"}"#, "search")]
#[case::arguments_only(r#"{"arguments":"{\"q\":\"cats\"}"}"#, "{\"q\":\"cats\"}")]
#[case::trimmed(r#"{"name":" search ","arguments":" { } "}"#, "search: { }")]
#[case::empty("{}", "")]
#[case::null_arguments(r#"{"name":"search","arguments":null}"#, "search")]
#[case::not_an_object("5", "")]
fn tool_intent_joins_name_and_arguments(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(render_tool_intent(&function(source)), expected);
}

#[test]
fn tool_output_resolves_the_intent_of_its_tool_call_id() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"find the cats"},
            {"role":"assistant","tool_calls":[
                {"id":"call_other","function":{"name":"other","arguments":"{}"}},
                {"id":"call_1","function":{"name":"search","arguments":"{\"q\":\"cats\"}"}}
            ]},
            {"role":"tool","tool_call_id":"call_1","content":"a long tool output"}
        ]"#,
    );
    assert_eq!(
        query_for_target(&conversation, 2, "find the cats"),
        "search: {\"q\":\"cats\"}"
    );
}

#[test]
fn tool_output_without_a_matching_id_falls_back() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"find the cats"},
            {"role":"assistant","tool_calls":[
                {"id":"call_1","function":{"name":"search","arguments":"{}"}}
            ]},
            {"role":"tool","tool_call_id":"call_missing","content":"output"}
        ]"#,
    );
    assert_eq!(
        query_for_target(&conversation, 2, "find the cats"),
        "find the cats"
    );
}

#[test]
fn the_closest_assistant_turn_wins() {
    let conversation = messages(
        r#"[
            {"role":"assistant","tool_calls":[
                {"id":"call_1","function":{"name":"first","arguments":"{}"}}
            ]},
            {"role":"assistant","tool_calls":[
                {"id":"call_1","function":{"name":"second","arguments":"{}"}}
            ]},
            {"role":"tool","tool_call_id":"call_1","content":"output"}
        ]"#,
    );
    assert_eq!(query_for_target(&conversation, 2, "fallback"), "second: {}");
}

#[test]
fn legacy_function_call_resolves_the_intent_by_name() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"look it up"},
            {"role":"assistant","function_call":{"name":"lookup","arguments":"{\"id\":7}"}},
            {"role":"function","name":"lookup","content":"the row"}
        ]"#,
    );
    assert_eq!(
        query_for_target(&conversation, 2, "look it up"),
        "lookup: {\"id\":7}"
    );
}

#[test]
fn legacy_function_call_with_a_different_name_falls_back() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"look it up"},
            {"role":"assistant","function_call":{"name":"lookup","arguments":"{\"id\":7}"}},
            {"role":"function","name":"other","content":"the row"}
        ]"#,
    );
    assert_eq!(
        query_for_target(&conversation, 2, "look it up"),
        "look it up"
    );
}

#[test]
fn a_user_message_uses_the_fallback_query() {
    let conversation = messages(r#"[{"role":"user","content":"compress me"}]"#);
    assert_eq!(
        query_for_target(&conversation, 0, "compress me"),
        "compress me"
    );
}

#[test]
fn a_named_user_message_never_takes_a_function_call_intent() {
    let conversation = messages(
        r#"[
            {"role":"assistant","content":null,"function_call":{"name":"lookup","arguments":"{}"}},
            {"role":"user","name":"lookup","content":"compress me"}
        ]"#,
    );
    assert_eq!(
        query_for_target(&conversation, 1, "the fallback"),
        "the fallback"
    );
}

#[test]
fn a_target_outside_the_conversation_uses_the_fallback_query() {
    let conversation = messages(r#"[{"role":"user","content":"hello"}]"#);
    assert_eq!(query_for_target(&conversation, 4, "hello"), "hello");
}

#[test]
fn an_empty_tool_intent_does_not_stop_the_search() {
    let conversation = messages(
        r#"[
            {"role":"assistant","function_call":{"name":"lookup","arguments":"{}"}},
            {"role":"assistant","tool_calls":[{"id":"call_1","function":{}}]},
            {"role":"tool","tool_call_id":"call_1","name":"lookup","content":"output"}
        ]"#,
    );
    assert_eq!(query_for_target(&conversation, 2, "fallback"), "lookup: {}");
}

#[test]
fn the_fallback_query_is_the_last_user_message() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","content":"reply"},
            {"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}
        ]"#,
    );
    assert_eq!(
        fallback_query(&conversation),
        ("a\n\nb".to_owned(), Some(2))
    );
}

#[test]
fn the_fallback_query_is_absent_without_a_user_message() {
    let conversation = messages(r#"[{"role":"system","content":"rules"}]"#);
    assert_eq!(fallback_query(&conversation), (String::new(), None));
}
