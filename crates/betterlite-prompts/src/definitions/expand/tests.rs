#![cfg(test)]

use super::unpack_defs;
use crate::arguments::json::{Json, Object};
use crate::definitions::error::DefinitionsError;
use rstest::rstest;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn object(text: &str) -> Object {
    match json(text) {
        Json::Object(map) => map,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("test fixtures only build objects")
        }
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Object(Object),
    BudgetExceeded(usize),
}

fn run(schema: Object, defs: &Object, max_inlined_bytes: Option<usize>) -> Outcome {
    match unpack_defs(schema, defs, max_inlined_bytes) {
        Ok(result) => Outcome::Object(result),
        Err(DefinitionsError::ExpansionBudgetExceeded { limit }) => Outcome::BudgetExceeded(limit),
    }
}

#[rstest]
#[case::defs_ref_expands_a_local_reference(
    object(r##"{"a": {"$ref": "#/$defs/Foo"}}"##),
    object(r#"{"Foo": {"type": "string"}}"#),
    None,
    Outcome::Object(object(r#"{"a": {"type": "string"}}"#))
)]
#[case::missing_ref_is_left_untouched(
    object(r##"{"a": {"$ref": "#/$defs/Missing"}}"##),
    object("{}"),
    None,
    Outcome::Object(object(r##"{"a": {"$ref": "#/$defs/Missing"}}"##))
)]
#[case::external_ref_is_never_fetched(
    object(r#"{"a": {"$ref": "https://example.com/other.json"}}"#),
    object("{}"),
    None,
    Outcome::Object(object(r#"{"a": {"$ref": "https://example.com/other.json"}}"#))
)]
#[case::recursive_ref_stops_at_the_cycle(
    object(r##"{"$ref": "#/$defs/Foo"}"##),
    object(r##"{"Foo": {"type": "object", "properties": {"child": {"$ref": "#/$defs/Foo"}}}}"##),
    None,
    Outcome::Object(object(
        r##"{"type": "object", "properties": {"child": {"$ref": "#/$defs/Foo"}}}"##
    ))
)]
#[case::nested_defs_shadow_the_parent_defs_table(
    object(
        r##"{"$defs": {"Foo": {"type": "string"}}, "a": {"$defs": {"Foo": {"type": "number"}}, "b": {"$ref": "#/$defs/Foo"}}}"##
    ),
    object("{}"),
    None,
    Outcome::Object(object(
        r#"{"$defs": {"Foo": {"type": "string"}}, "a": {"$defs": {"Foo": {"type": "number"}}, "b": {"type": "number"}}}"#
    ))
)]
#[case::root_defs_merge_applies_even_when_the_root_is_a_ref(
    object(r##"{"$defs": {"Foo": {"type": "string"}}, "$ref": "#/$defs/Foo"}"##),
    object("{}"),
    None,
    Outcome::Object(object(r#"{"type": "string"}"#))
)]
#[case::non_string_ref_value_is_left_unresolved(
    object(r#"{"a": {"$ref": 123}}"#),
    object("{}"),
    None,
    Outcome::Object(object(r#"{"a": {"$ref": 123}}"#))
)]
#[case::a_root_only_ref_resolving_to_a_scalar_falls_back_to_the_original_schema(
    object(r##"{"$ref": "#/$defs/Scalar"}"##),
    object(r#"{"Scalar": 1}"#),
    None,
    Outcome::Object(object(r##"{"$ref": "#/$defs/Scalar"}"##))
)]
#[case::the_byte_budget_rejects_an_oversized_target(
    object(r##"{"a": {"$ref": "#/$defs/Big"}}"##),
    object(&format!(r#"{{"Big": {{"description": "{}"}}}}"#, "x".repeat(50))),
    Some(10),
    Outcome::BudgetExceeded(10)
)]
#[case::the_byte_budget_accepts_a_target_that_exactly_fills_it(
    object(r##"{"a": {"$ref": "#/$defs/Foo"}}"##),
    object(r#"{"Foo": {"type": "string"}}"#),
    Some(18),
    Outcome::Object(object(r#"{"a": {"type": "string"}}"#))
)]
#[case::refs_inside_a_list_item_are_expanded(
    object(r##"{"a": [{"$ref": "#/$defs/Foo"}, {"$ref": "#/$defs/Bar"}]}"##),
    object(r#"{"Foo": {"type": "string"}, "Bar": {"type": "number"}}"#),
    None,
    Outcome::Object(object(r#"{"a": [{"type": "string"}, {"type": "number"}]}"#))
)]
fn schema_definitions_behavior_tests_behavior(
    #[case] schema: Object,
    #[case] defs: Object,
    #[case] max_inlined_bytes: Option<usize>,
    #[case] expected: Outcome,
) {
    assert_eq!(run(schema, &defs, max_inlined_bytes), expected);
}
