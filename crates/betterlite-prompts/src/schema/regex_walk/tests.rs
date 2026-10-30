#![cfg(test)]

use std::{fs, path::PathBuf};

use crate::arguments::json::{Json, Object};
use crate::schema::{
    drop_non_python_regex_patterns, flatten_and_drop, flatten_top_level,
    tool_with_sanitized_parameters,
};
use rstest::rstest;

fn object(text: &str) -> Object {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(value) => value,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("fixture must be an object")
        }
    }
}

#[rstest]
#[case::top_level_combinators(
    object(r#"{"anyOf":[{"type":"object","properties":{"a":{"type":"string"}}},{"type":"object","properties":{"b":{"type":"number"}}}]}"#),
    object(r#"{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"number"}}}"#)
)]
#[case::required_branch_merge(
    object(r#"{"allOf":[{"type":"object","required":["a"]},{"type":"object","required":["b"]}]}"#),
    object(r#"{"type":"object","properties":{},"required":["a","b"]}"#)
)]
#[case::unmergeable_branch(
    object(
        r#"{"oneOf":[{"type":"string"},{"type":"object","properties":{"a":{"type":"string"}}}]}"#
    ),
    object(
        r#"{"oneOf":[{"type":"string"},{"type":"object","properties":{"a":{"type":"string"}}}]}"#
    )
)]
#[case::local_ref_branch(
    object(r##"{"$defs":{"entry":{"type":"object","properties":{"id":{"type":"string"}}}},"anyOf":[{"$ref":"#/$defs/entry"}]}"##),
    object(r#"{"$defs":{"entry":{"type":"object","properties":{"id":{"type":"string"}}}},"type":"object","properties":{"id":{"type":"string"}}}"#)
)]
#[case::invalid_combinator_shape(
    object(r#"{"type":"object","allOf":{"type":"object"},"properties":{}}"#),
    object(r#"{"type":"object","allOf":{"type":"object"},"properties":{}}"#)
)]
fn tool_schemas_tool_schema_tests_behavior(#[case] schema: Object, #[case] expected: Object) {
    assert_eq!(flatten_top_level(&schema), expected);
}

#[test]
fn nested_non_python_regex_is_removed_without_touching_data() {
    let schema = object(
        r#"{"type":"object","properties":{"x":{"type":"string","pattern":"\\p{Cc}","default":{"pattern":"\\p{Cc}"}}}}"#,
    );
    let expected = object(
        r#"{"type":"object","properties":{"x":{"type":"string","default":{"pattern":"\\p{Cc}"}}}}"#,
    );
    assert_eq!(drop_non_python_regex_patterns(&schema), expected);
}

#[test]
fn escaped_regex_marker_is_preserved() {
    let schema = object(r#"{"type":"object","pattern":"\\\\p{Cc}"}"#);
    assert_eq!(drop_non_python_regex_patterns(&schema), schema);
}

#[rstest]
#[case::pattern_properties(
    r#"{"type":"object","patternProperties":{"\\p{Cc}":{"type":"string"},"^[a-z]+$":{"type":"number"}}}"#,
    r#"{"type":"object","patternProperties":{"^[a-z]+$":{"type":"number"}}}"#
)]
#[case::data_pattern_is_preserved(
    r#"{"type":"object","properties":{"x":{"type":"string","default":{"pattern":"\\p{Cc}"}}}}"#,
    r#"{"type":"object","properties":{"x":{"type":"string","default":{"pattern":"\\p{Cc}"}}}}"#
)]
fn tool_schemas_tool_schema_tests_regex_behavior(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        drop_non_python_regex_patterns(&object(input)),
        object(expected)
    );
}

#[test]
fn tool_schemas_tool_schema_tests_sanitizes_only_parameters() {
    let tool = object(
        r#"{"type":"function","function":{"name":"lookup","parameters":{"type":"object","pattern":"\\p{Cc}"}}}"#,
    );
    let expected = object(
        r#"{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}"#,
    );
    assert_eq!(
        tool_with_sanitized_parameters(tool, flatten_and_drop),
        expected
    );
}

#[rstest]
fn replays_the_tool_schema_corpus(#[files("fuzz/corpus/tool_schema/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let Ok(schema) = serde_json::from_slice::<Object>(&bytes) else {
        return;
    };
    drop(flatten_and_drop(&schema));
}
