use rstest::rstest;

use crate::arguments::json::Json;
use crate::bedrock_tool_results::error::BedrockToolResultsError;
use crate::bedrock_tool_results::invoke::tool_call_invoke_blocks;

use super::prelude::{FixedCachePoints, as_json, element, expected, field, json};

fn invoke(calls: &[Json]) -> Json {
    let blocks = tool_call_invoke_blocks(calls, None, &FixedCachePoints::accepting()).unwrap();
    as_json(&blocks)
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_function_call_becomes_a_tool_use_block() {
    assert_eq!(
        invoke(&[json(
            r#"{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Boston\"}"}}"#
        )]),
        expected(
            r#"[{"toolUse":{"input":{"location":"Boston"},"name":"get_weather","toolUseId":"call_1"}}]"#
        )
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_several_calls_keep_their_order() {
    assert_eq!(
        invoke(&[
            json(r#"{"id":"call_1","function":{"name":"first","arguments":"{}"}}"#),
            json(r#"{"id":"call_2","function":{"name":"second","arguments":"{}"}}"#),
        ]),
        expected(
            r#"[{"toolUse":{"input":{},"name":"first","toolUseId":"call_1"}},{"toolUse":{"input":{},"name":"second","toolUseId":"call_2"}}]"#
        )
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_call_without_a_function_is_skipped() {
    assert_eq!(
        invoke(&[
            json(r#"{"id":"call_1","type":"web_search"}"#),
            json(r#"{"id":"call_2","function":{"name":"kept","arguments":"{}"}}"#),
        ]),
        expected(r#"[{"toolUse":{"input":{},"name":"kept","toolUseId":"call_2"}}]"#)
    );
}

#[rstest]
#[case::absent(r#"{"id":"call_1","function":{"name":"n"}}"#)]
#[case::empty(r#"{"id":"call_1","function":{"name":"n","arguments":""}}"#)]
#[case::blank(r#"{"id":"call_1","function":{"name":"n","arguments":"   "}}"#)]
#[case::json_null(r#"{"id":"call_1","function":{"name":"n","arguments":null}}"#)]
#[case::empty_list(r#"{"id":"call_1","function":{"name":"n","arguments":[]}}"#)]
#[case::empty_object(r#"{"id":"call_1","function":{"name":"n","arguments":{}}}"#)]
#[case::zero(r#"{"id":"call_1","function":{"name":"n","arguments":0}}"#)]
#[case::bool_false(r#"{"id":"call_1","function":{"name":"n","arguments":false}}"#)]
#[case::json_string(r#"{"id":"call_1","function":{"name":"n","arguments":"\"\""}}"#)]
#[case::not_json(r#"{"id":"call_1","function":{"name":"n","arguments":"not json"}}"#)]
#[case::truncated(r#"{"id":"call_1","function":{"name":"n","arguments":"{\"a\":"}}"#)]
fn bedrock_tool_results_behavior_tests_behavior_unusable_arguments_become_an_empty_input(
    #[case] call: &str,
) {
    assert_eq!(
        invoke(&[json(call)]),
        expected(r#"[{"toolUse":{"input":{},"name":"n","toolUseId":"call_1"}}]"#)
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_concatenated_arguments_become_one_block_each() {
    assert_eq!(
        invoke(&[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"{\"a\":1}{\"b\":2}{\"c\":3}"}}"#
        )]),
        expected(
            r#"[{"toolUse":{"input":{"a":1},"name":"n","toolUseId":"call_1"}},{"toolUse":{"input":{"b":2},"name":"n","toolUseId":"call_1_1"}},{"toolUse":{"input":{"c":3},"name":"n","toolUseId":"call_1_2"}}]"#
        )
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_concatenated_non_objects_become_empty_inputs() {
    assert_eq!(
        invoke(&[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"1 2"}}"#
        )]),
        expected(
            r#"[{"toolUse":{"input":{},"name":"n","toolUseId":"call_1"}},{"toolUse":{"input":{},"name":"n","toolUseId":"call_1_1"}}]"#
        )
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_broken_concatenated_tail_is_discarded() {
    assert_eq!(
        invoke(&[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"{\"a\":1}{\"b\":"}}"#
        )]),
        expected(r#"[{"toolUse":{"input":{"a":1},"name":"n","toolUseId":"call_1"}}]"#)
    );
}

#[rstest]
#[case::invalid_characters("weird name!", "weird_name_")]
#[case::leading_digit("9lives", "a9lives")]
#[case::leading_dash("-abc", "a-abc")]
#[case::dot("a.b", "a_b")]
#[case::empty("", "")]
fn bedrock_tool_results_behavior_tests_behavior_tool_names_are_normalized(
    #[case] name: &str,
    #[case] normalized: &str,
) {
    let encoded = serde_json::to_string(name).unwrap();
    let call = json(&format!(
        r#"{{"id":"call_1","function":{{"name":{encoded},"arguments":"{{}}"}}}}"#
    ));
    let blocks =
        as_json(&tool_call_invoke_blocks(&[call], None, &FixedCachePoints::accepting()).unwrap());
    assert_eq!(
        field(field(element(&blocks, 0), "toolUse"), "name").as_str(),
        Some(normalized)
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_tool_ids_are_sanitized() {
    let blocks = invoke(&[json(
        r#"{"id":"call/1","function":{"name":"n","arguments":"{}"}}"#,
    )]);
    let tool_use_id = field(field(element(&blocks, 0), "toolUse"), "toolUseId")
        .as_str()
        .unwrap();
    assert!(tool_use_id.starts_with("call_1_"), "{tool_use_id}");
    assert_eq!(tool_use_id.len(), 15);
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_cache_control_adds_a_cache_point_block() {
    assert_eq!(
        invoke(&[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"{}"},"cache_control":{"type":"ephemeral"}}"#
        )]),
        expected(
            r#"[{"toolUse":{"input":{},"name":"n","toolUseId":"call_1"}},{"cachePoint":{"type":"default"}}]"#
        )
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_null_cache_control_adds_nothing() {
    let cache_points = FixedCachePoints::accepting();
    let blocks = tool_call_invoke_blocks(
        &[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"{}"},"cache_control":null}"#,
        )],
        None,
        &cache_points,
    )
    .unwrap();
    assert_eq!(
        as_json(&blocks),
        expected(r#"[{"toolUse":{"input":{},"name":"n","toolUseId":"call_1"}}]"#)
    );
    assert!(cache_points.calls().is_empty());
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_model_that_refuses_cache_points_adds_nothing() {
    let cache_points = FixedCachePoints::denying();
    let blocks = tool_call_invoke_blocks(
        &[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"{}"},"cache_control":{"type":"ephemeral"}}"#,
        )],
        Some("openai.gpt-5"),
        &cache_points,
    )
    .unwrap();
    assert_eq!(
        as_json(&blocks),
        expected(r#"[{"toolUse":{"input":{},"name":"n","toolUseId":"call_1"}}]"#)
    );
    assert_eq!(cache_points.calls().len(), 1);
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_the_cache_point_reader_sees_the_control_and_model()
{
    let cache_points = FixedCachePoints::accepting();
    let _blocks = tool_call_invoke_blocks(
        &[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"{}"},"cache_control":{"type":"ephemeral","ttl":"1h"}}"#,
        )],
        Some("anthropic.claude-4-5"),
        &cache_points,
    )
    .unwrap();
    assert_eq!(
        cache_points.calls(),
        [(
            json(r#"{"type":"ephemeral","ttl":"1h"}"#),
            Some("anthropic.claude-4-5".to_owned())
        )]
    );
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_split_blocks_come_before_the_cache_point() {
    assert_eq!(
        invoke(&[json(
            r#"{"id":"call_1","function":{"name":"n","arguments":"{\"a\":1}{\"b\":2}"},"cache_control":{"type":"ephemeral"}}"#
        )]),
        expected(
            r#"[{"toolUse":{"input":{"a":1},"name":"n","toolUseId":"call_1"}},{"toolUse":{"input":{"b":2},"name":"n","toolUseId":"call_1_1"}},{"cachePoint":{"type":"default"}}]"#
        )
    );
}

fn failure(call: &str) -> BedrockToolResultsError {
    tool_call_invoke_blocks(&[json(call)], None, &FixedCachePoints::accepting()).unwrap_err()
}

#[rstest]
#[case::not_an_object("5", "ToolCallNotObject")]
#[case::null_function(r#"{"id":"c","function":null}"#, "FunctionNotObject")]
#[case::string_function(r#"{"id":"c","function":"f"}"#, "FunctionNotObject")]
#[case::missing_id(r#"{"function":{"name":"n"}}"#, "ToolCallWithoutId")]
#[case::numeric_id(r#"{"id":5,"function":{"name":"n"}}"#, "ToolCallIdNotText")]
#[case::numeric_name(r#"{"id":"c","function":{"name":5}}"#, "FunctionNameNotText")]
#[case::numeric_arguments(r#"{"id":"c","function":{"arguments":5}}"#, "ArgumentsNotText")]
#[case::list_arguments(r#"{"id":"c","function":{"arguments":[1]}}"#, "ArgumentsNotText")]
#[case::object_arguments(r#"{"id":"c","function":{"arguments":{"a":1}}}"#, "ArgumentsNotText")]
fn bedrock_tool_results_behavior_tests_behavior_an_unusable_tool_call_names_its_failure(
    #[case] call: &str,
    #[case] variant: &str,
) {
    let error = failure(call);
    assert_eq!(format!("{error:?}").split([' ', '{']).next(), Some(variant));
}

#[test]
fn bedrock_tool_results_behavior_tests_behavior_a_null_function_name_becomes_empty() {
    assert_eq!(
        invoke(&[json(
            r#"{"id":"call_1","function":{"name":null,"arguments":"{}"}}"#
        )]),
        expected(r#"[{"toolUse":{"input":{},"name":"","toolUseId":"call_1"}}]"#)
    );
}
