#![cfg(test)]

use super::flatten_and_drop;
use crate::arguments::json::{Json, Object};
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
#[case::preserve_invalid_combinator_alongside_valid(
    object(r#"{"type":"object","allOf":{},"anyOf":[]}"#),
    object(r#"{"type":"object","allOf":{},"properties":{}}"#)
)]
#[case::required_intersection(
    object(r#"{"required":["root",7],"anyOf":[{"type":"object","required":["a","b"]},{"type":"object","required":["b","c"]}]}"#),
    object(r#"{"type":"object","properties":{},"required":["b","root"]}"#)
)]
#[case::property_precedence(
    object(r#"{"properties":{"a":1},"allOf":[{"properties":{"a":2,"b":3}},{"properties":{"a":4,"b":5}}]}"#),
    object(r#"{"type":"object","properties":{"a":1,"b":3}}"#)
)]
#[case::rejected_top_level_keys(
    object(r#"{"type":"object","enum":[],"const":{},"not":{},"title":"kept"}"#),
    object(r#"{"type":"object","properties":{},"title":"kept"}"#)
)]
#[case::scalar_passthrough(
    object(r#"{"type":"string","enum":["a"]}"#),
    object(r#"{"type":"string","enum":["a"]}"#)
)]
#[case::boolean_branch(
    object(r#"{"type":"object","anyOf":[false]}"#),
    object(r#"{"type":"object","anyOf":[false]}"#)
)]
#[case::empty_branch(
    object(r#"{"type":"object","anyOf":[{}]}"#),
    object(r#"{"type":"object","properties":{}}"#)
)]
#[case::non_string_ref(
    object(r#"{"type":"object","anyOf":[{"$ref":false}]}"#),
    object(r#"{"type":"object","properties":{}}"#)
)]
#[case::cycle(
    object(r##"{"$defs":{"a":{"$ref":"#/$defs/a"}},"anyOf":[{"$ref":"#/$defs/a"}]}"##),
    object(r##"{"$defs":{"a":{"$ref":"#/$defs/a"}},"anyOf":[{"$ref":"#/$defs/a"}]}"##)
)]
#[case::external_reference(
    object(r#"{"type":"object","allOf":[{"$ref":"https://example.test/schema"}]}"#),
    object(r#"{"type":"object","allOf":[{"$ref":"https://example.test/schema"}]}"#)
)]
#[case::invalid_selector(
    object(r##"{"type":"object","allOf":[{"$ref":"#/$defs/missing"}]}"##),
    object(r##"{"type":"object","allOf":[{"$ref":"#/$defs/missing"}]}"##)
)]
#[case::legacy_definitions(
    object(r##"{"definitions":{"a":{"type":"object"}},"anyOf":[{"$ref":"#/definitions/a"},{"$ref":"#/definitions/a"}]}"##),
    object(r#"{"definitions":{"a":{"type":"object"}},"type":"object","properties":{}}"#)
)]
#[case::python_lookahead(
    object(r#"{"type":"string","pattern":"a(?=b)"}"#),
    object(r#"{"type":"string","pattern":"a(?=b)"}"#)
)]
#[case::python_backreference(
    object(r#"{"type":"string","pattern":"(a)\\1"}"#),
    object(r#"{"type":"string","pattern":"(a)\\1"}"#)
)]
#[case::python_literal_braces(
    object(r#"{"type":"string","pattern":"a{word}"}"#),
    object(r#"{"type":"string","pattern":"a{word}"}"#)
)]
fn tool_schemas_tool_schema_tests_behavior(#[case] schema: Object, #[case] expected: Object) {
    assert_eq!(flatten_and_drop(&schema), expected);
}

#[rstest]
#[case::exact_depth_limit(30, true)]
#[case::past_depth_limit(31, false)]
fn tool_schemas_tool_schema_tests_depth(#[case] count: usize, #[case] accepted: bool) {
    let definitions = (0..count)
        .map(|index| {
            let next = index + 1;
            format!(r##""d{index}":{{"$ref":"#/$defs/d{next}"}},"##)
        })
        .chain(std::iter::once(format!(
            r#""d{count}":{{"type":"object","properties":{{"ok":{{"type":"boolean"}}}}}}"#
        )))
        .collect::<String>();
    let input = format!(r##"{{"$defs":{{{definitions}}},"anyOf":[{{"$ref":"#/$defs/d0"}}]}}"##);
    let flattened = flatten_and_drop(&object(&input));
    assert_eq!(
        flattened.get("type").and_then(Json::as_str) == Some("object"),
        accepted
    );
}
