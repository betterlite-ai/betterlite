#![cfg(test)]

use super::{salvage_concatenated_tool_arguments, split_concatenated_json_objects};
use crate::arguments::json::{Json, Object};
use rstest::rstest;
use std::fmt::Write as _;

#[derive(Clone, Copy)]
enum Operation {
    Split,
    Salvage,
}

fn object(text: &str) -> Object {
    match serde_json::from_str(text).expect("fixture literal is valid JSON") {
        Json::Object(map) => map,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("test fixtures only build objects")
        }
    }
}

fn run(operation: Operation, raw: &str) -> Vec<Object> {
    match operation {
        Operation::Split => split_concatenated_json_objects(raw),
        Operation::Salvage => salvage_concatenated_tool_arguments(raw),
    }
}

#[rstest]
#[case::split_of_empty_string_yields_no_objects(Operation::Split, "", vec![])]
#[case::split_of_a_single_object(Operation::Split, "{\"a\":1}", vec!["{\"a\": 1}"])]
#[case::split_of_two_concatenated_objects(
    Operation::Split,
    "{\"a\":1}{\"b\":2}",
    vec!["{\"a\": 1}", "{\"b\": 2}"]
)]
#[case::split_skips_whitespace_between_objects(
    Operation::Split,
    "{\"a\":1}  \n  {\"b\":2}",
    vec!["{\"a\": 1}", "{\"b\": 2}"]
)]
#[case::split_turns_a_non_dict_value_into_an_empty_object(
    Operation::Split,
    "1 {\"a\":1}",
    vec!["{}", "{\"a\": 1}"]
)]
#[case::split_discards_only_a_malformed_tail(
    Operation::Split,
    "{\"a\":1} not json",
    vec!["{\"a\": 1}"]
)]
#[case::split_of_fully_malformed_input_yields_no_objects(Operation::Split, "not json at all", vec![])]
#[case::salvage_of_empty_string_yields_no_objects(Operation::Salvage, "", vec![])]
#[case::salvage_keeps_a_single_object(Operation::Salvage, "{\"a\":1}", vec!["{\"a\": 1}"])]
#[case::salvage_collapses_identical_repeats_to_one(
    Operation::Salvage,
    "{\"a\":1}{\"a\":1}{\"a\":1}",
    vec!["{\"a\": 1}"]
)]
#[case::salvage_expands_the_run_once_a_distinct_object_appears(
    Operation::Salvage,
    "{\"a\":1}{\"a\":1}{\"a\":1}{\"b\":2}",
    vec!["{\"a\": 1}", "{\"a\": 1}", "{\"a\": 1}", "{\"b\": 2}"]
)]
#[case::salvage_discards_everything_on_a_non_dict_value(Operation::Salvage, "{\"a\":1}1", vec![])]
#[case::salvage_discards_everything_on_a_malformed_tail(
    Operation::Salvage,
    "{\"a\":1} not json",
    vec![]
)]
#[case::salvage_of_fully_malformed_input_yields_no_objects(Operation::Salvage, "not json at all", vec![])]
fn tool_arguments_behavior_tests_behavior(
    #[case] operation: Operation,
    #[case] raw: &str,
    #[case] expected_literals: Vec<&str>,
) {
    let expected: Vec<Object> = expected_literals.into_iter().map(object).collect();
    assert_eq!(run(operation, raw), expected);
}

#[test]
fn more_than_the_cap_of_distinct_objects_yields_nothing() {
    let mut raw = String::new();
    for index in 0_u8..9_u8 {
        write!(raw, "{{\"n\":{index}}}").expect("write to String succeeds");
    }
    assert_eq!(
        salvage_concatenated_tool_arguments(&raw),
        Vec::<Object>::new()
    );
}

#[test]
fn exactly_the_cap_of_distinct_objects_is_kept() {
    let mut raw = String::new();
    for index in 0_u8..8_u8 {
        write!(raw, "{{\"n\":{index}}}").expect("write to String succeeds");
    }
    let expected: Vec<Object> = (0_u8..8_u8)
        .map(|index| object(&format!("{{\"n\": {index}}}")))
        .collect();
    assert_eq!(salvage_concatenated_tool_arguments(&raw), expected);
}
