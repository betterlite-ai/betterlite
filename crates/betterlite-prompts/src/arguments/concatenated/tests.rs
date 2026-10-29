#![cfg(test)]

use super::{salvage_concatenated_tool_arguments, split_concatenated_json_objects};
use rstest::rstest;
use serde_json::{Map, Value, json};

#[derive(Clone, Copy)]
enum Operation {
    Split,
    Salvage,
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => unreachable!("test fixtures only build objects"),
    }
}

fn run(operation: Operation, raw: &str) -> Vec<Map<String, Value>> {
    match operation {
        Operation::Split => split_concatenated_json_objects(raw),
        Operation::Salvage => salvage_concatenated_tool_arguments(raw),
    }
}

#[rstest]
#[case::split_of_empty_string_yields_no_objects(Operation::Split, "", vec![])]
#[case::split_of_a_single_object(Operation::Split, "{\"a\":1}", vec![json!({"a": 1})])]
#[case::split_of_two_concatenated_objects(
    Operation::Split,
    "{\"a\":1}{\"b\":2}",
    vec![json!({"a": 1}), json!({"b": 2})]
)]
#[case::split_skips_whitespace_between_objects(
    Operation::Split,
    "{\"a\":1}  \n  {\"b\":2}",
    vec![json!({"a": 1}), json!({"b": 2})]
)]
#[case::split_turns_a_non_dict_value_into_an_empty_object(
    Operation::Split,
    "1 {\"a\":1}",
    vec![json!({}), json!({"a": 1})]
)]
#[case::split_discards_only_a_malformed_tail(
    Operation::Split,
    "{\"a\":1} not json",
    vec![json!({"a": 1})]
)]
#[case::split_of_fully_malformed_input_yields_no_objects(Operation::Split, "not json at all", vec![])]
#[case::salvage_of_empty_string_yields_no_objects(Operation::Salvage, "", vec![])]
#[case::salvage_keeps_a_single_object(Operation::Salvage, "{\"a\":1}", vec![json!({"a": 1})])]
#[case::salvage_collapses_identical_repeats_to_one(
    Operation::Salvage,
    "{\"a\":1}{\"a\":1}{\"a\":1}",
    vec![json!({"a": 1})]
)]
#[case::salvage_expands_the_run_once_a_distinct_object_appears(
    Operation::Salvage,
    "{\"a\":1}{\"a\":1}{\"a\":1}{\"b\":2}",
    vec![json!({"a": 1}), json!({"a": 1}), json!({"a": 1}), json!({"b": 2})]
)]
#[case::salvage_discards_everything_on_a_non_dict_value(
    Operation::Salvage,
    "{\"a\":1}1",
    vec![]
)]
#[case::salvage_discards_everything_on_a_malformed_tail(
    Operation::Salvage,
    "{\"a\":1} not json",
    vec![]
)]
#[case::salvage_of_fully_malformed_input_yields_no_objects(Operation::Salvage, "not json at all", vec![])]
fn tool_arguments_behavior_tests_behavior(
    #[case] operation: Operation,
    #[case] raw: &str,
    #[case] expected: Vec<Value>,
) {
    let expected: Vec<Map<String, Value>> = expected.into_iter().map(object).collect();
    assert_eq!(run(operation, raw), expected);
}

#[test]
fn more_than_the_cap_of_distinct_objects_yields_nothing() {
    let mut raw = String::new();
    for index in 0..9 {
        raw.push_str(&format!("{{\"n\":{index}}}"));
    }
    assert_eq!(salvage_concatenated_tool_arguments(&raw), Vec::new());
}

#[test]
fn exactly_the_cap_of_distinct_objects_is_kept() {
    let mut raw = String::new();
    for index in 0..8 {
        raw.push_str(&format!("{{\"n\":{index}}}"));
    }
    let expected: Vec<Map<String, Value>> = (0..8).map(|index| object(json!({"n": index}))).collect();
    assert_eq!(salvage_concatenated_tool_arguments(&raw), expected);
}
