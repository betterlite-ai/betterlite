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
    Outcome::Object(object(r#"{"a": {"type": "string"}}"#)),
    Some(vec!["a"])
)]
#[case::missing_ref_is_left_untouched(
    object(r##"{"a": {"$ref": "#/$defs/Missing"}}"##),
    object("{}"),
    None,
    Outcome::Object(object(r##"{"a": {"$ref": "#/$defs/Missing"}}"##)),
    None
)]
#[case::external_ref_is_never_fetched(
    object(r#"{"a": {"$ref": "https://example.com/other.json"}}"#),
    object("{}"),
    None,
    Outcome::Object(object(r#"{"a": {"$ref": "https://example.com/other.json"}}"#)),
    None
)]
#[case::recursive_ref_stops_at_the_cycle(
    object(r##"{"$ref": "#/$defs/Foo"}"##),
    object(r##"{"Foo": {"type": "object", "properties": {"child": {"$ref": "#/$defs/Foo"}}}}"##),
    None,
    Outcome::Object(object(
        r##"{"type": "object", "properties": {"child": {"$ref": "#/$defs/Foo"}}}"##
    )),
    None
)]
#[case::nested_defs_shadow_the_parent_defs_table(
    object(
        r##"{"$defs": {"Foo": {"type": "string"}}, "a": {"$defs": {"Foo": {"type": "number"}}, "b": {"$ref": "#/$defs/Foo"}}}"##
    ),
    object("{}"),
    None,
    Outcome::Object(object(
        r#"{"$defs": {"Foo": {"type": "string"}}, "a": {"$defs": {"Foo": {"type": "number"}}, "b": {"type": "number"}}}"#
    )),
    Some(vec!["$defs", "a"])
)]
#[case::root_defs_merge_applies_even_when_the_root_is_a_ref(
    object(r##"{"$defs": {"Foo": {"type": "string"}}, "$ref": "#/$defs/Foo"}"##),
    object("{}"),
    None,
    Outcome::Object(object(r#"{"type": "string"}"#)),
    None
)]
#[case::non_string_ref_value_is_left_unresolved(
    object(r#"{"a": {"$ref": 123}}"#),
    object("{}"),
    None,
    Outcome::Object(object(r#"{"a": {"$ref": 123}}"#)),
    None
)]
#[case::a_root_only_ref_resolving_to_a_scalar_falls_back_to_the_original_schema(
    object(r##"{"$ref": "#/$defs/Scalar"}"##),
    object(r#"{"Scalar": 1}"#),
    None,
    Outcome::Object(object(r##"{"$ref": "#/$defs/Scalar"}"##)),
    None
)]
#[case::the_byte_budget_rejects_an_oversized_target(
    object(r##"{"a": {"$ref": "#/$defs/Big"}}"##),
    object(&format!(r#"{{"Big": {{"description": "{}"}}}}"#, "x".repeat(50))),
    Some(10),
    Outcome::BudgetExceeded(10),
    None
)]
#[case::the_byte_budget_accepts_a_target_that_exactly_fills_it(
    object(r##"{"a": {"$ref": "#/$defs/Foo"}}"##),
    object(r#"{"Foo": {"type": "string"}}"#),
    Some(18),
    Outcome::Object(object(r#"{"a": {"type": "string"}}"#)),
    None
)]
#[case::refs_inside_a_list_item_are_expanded(
    object(r##"{"a": [{"$ref": "#/$defs/Foo"}, {"$ref": "#/$defs/Bar"}]}"##),
    object(r#"{"Foo": {"type": "string"}, "Bar": {"type": "number"}}"#),
    None,
    Outcome::Object(object(r#"{"a": [{"type": "string"}, {"type": "number"}]}"#)),
    None
)]
#[case::a_non_object_root_defs_value_is_ignored(
    object(r##"{"$defs": "not an object", "a": {"$ref": "#/$defs/Foo"}}"##),
    object("{}"),
    None,
    Outcome::Object(object(r##"{"$defs": "not an object", "a": {"$ref": "#/$defs/Foo"}}"##)),
    None
)]
#[case::a_non_object_defs_value_on_a_resolved_ref_target_is_ignored(
    object(r##"{"a": {"$ref": "#/$defs/Foo"}}"##),
    object(r#"{"Foo": {"$defs": "not an object", "type": "string"}}"#),
    None,
    Outcome::Object(object(r#"{"a": {"$defs": "not an object", "type": "string"}}"#)),
    None
)]
#[case::a_non_object_definitions_value_on_a_plain_dict_is_ignored(
    object(r##"{"a": {"definitions": "not an object", "b": {"$ref": "#/$defs/Foo"}}}"##),
    object("{}"),
    None,
    Outcome::Object(object(
        r##"{"a": {"definitions": "not an object", "b": {"$ref": "#/$defs/Foo"}}}"##
    )),
    None
)]
fn schema_definitions_behavior_tests_behavior(
    #[case] schema: Object,
    #[case] defs: Object,
    #[case] max_inlined_bytes: Option<usize>,
    #[case] expected: Outcome,
    #[case] expected_keys: Option<Vec<&str>>,
) {
    let actual = run(schema, &defs, max_inlined_bytes);
    assert_eq!(actual, expected);
    if let Some(expected_order) = expected_keys {
        let Outcome::Object(result) = actual else {
            panic!("key order assertions require a successful object result");
        };
        let actual_keys: Vec<&str> = result.keys().map(String::as_str).collect();
        assert_eq!(actual_keys, expected_order);
    }
}
