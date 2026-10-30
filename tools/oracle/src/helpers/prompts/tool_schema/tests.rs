#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::Value;

#[rstest]
#[case::top_level_combinators(
    r#"{"mode":"flatten_top_level","schema":{"anyOf":[{"type":"object","properties":{"a":{"type":"string"}},"required":["a"]},{"type":"object","properties":{"b":{"type":"number"}},"required":["b"]}]}}"#,
    serde_json::json!({"type":"object","properties":{"a":{"type":"string"},"b":{"type":"number"}}})
)]
#[case::non_python_regex_is_removed(
    r#"{"mode":"drop_non_python_regex_patterns","schema":{"type":"object","pattern":"\\p{Cc}","properties":{"x":{"type":"string","pattern":"^[a-z]+$"}}}}"#,
    serde_json::json!({"type":"object","properties":{"x":{"type":"string","pattern":"^[a-z]+$"}}})
)]
fn tool_schemas_tool_schema_tests_behavior(#[case] wire: &str, #[case] expected: Value) {
    let input: Input = serde_json::from_str(wire).unwrap();
    assert_eq!(Op::run(&input).unwrap(), expected);
}

#[test]
fn unmergeable_branch_is_preserved() {
    let input: Input = serde_json::from_str(
        r#"{"mode":"flatten_top_level","schema":{"oneOf":[{"type":"string"},{"type":"object","properties":{}}]}}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(output["oneOf"].as_array().map(Vec::len), Some(2));
}
