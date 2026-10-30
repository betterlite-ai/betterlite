#![cfg(test)]

use crate::arguments::json::{Json, Object};
use crate::schema::drop_non_python_regex_patterns;
use crate::schema::flatten::{flatten_and_drop, tool_with_sanitized_parameters};
use rstest::rstest;
use std::{fs, path::PathBuf};

fn object(text: &str) -> Object {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(value) => value,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("fixture must be an object")
        }
    }
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
#[case::nested_regex(
    r#"{"properties":{"x":{"pattern":"\\p{Cc}","default":{"pattern":"\\p{Cc}"}}}}"#,
    r#"{"properties":{"x":{"default":{"pattern":"\\p{Cc}"}}}}"#
)]
#[case::object_items(r#"{"items":{"pattern":"\\p{Cc}"}}"#, r#"{"items":{}}"#)]
#[case::schema_applicators(
    r#"{"$defs":{"x":{"pattern":"\\P{Cc}"}},"allOf":[{"pattern":"\\z"}],"additionalProperties":{"pattern":"\\p{Cc}"},"properties":{"x":{"pattern":"\\p{Cc}"}}}"#,
    r#"{"$defs":{"x":{}},"allOf":[{}],"additionalProperties":{},"properties":{"x":{}}}"#
)]
#[case::escaped_marker(r#"{"pattern":"\\\\p{Cc}"}"#, r#"{"pattern":"\\\\p{Cc}"}"#)]
#[case::bare_property(r#"{"pattern":"\\pL"}"#, "{}")]
#[case::bare_negated_property(r#"{"pattern":"\\PL"}"#, "{}")]
#[case::escaped_bare_property(r#"{"pattern":"\\\\pL"}"#, r#"{"pattern":"\\\\pL"}"#)]
#[case::escaped_bare_negated_property(r#"{"pattern":"\\\\PL"}"#, r#"{"pattern":"\\\\PL"}"#)]
#[case::bare_pattern_properties(
    r#"{"patternProperties":{"\\pL":{},"\\PL":{},"\\\\pL":{},"\\\\PL":{}}}"#,
    r#"{"patternProperties":{"\\\\pL":{},"\\\\PL":{}}}"#
)]
#[case::invalid_regex(r#"{"pattern":"["}"#, "{}")]
#[case::mid_pattern_flag(r#"{"pattern":"a(?i)b"}"#, r#"{"pattern":"a(?i)b"}"#)]
#[case::search_anchor(r#"{"pattern":"\\G"}"#, r#"{"pattern":"\\G"}"#)]
#[case::stacked_repeats(r#"{"pattern":"a{1,2}{3}"}"#, r#"{"pattern":"a{1,2}{3}"}"#)]
fn tool_schemas_tool_schema_tests_behavior(#[case] input: &str, #[case] expected: &str) {
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
