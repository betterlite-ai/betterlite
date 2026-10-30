#![cfg(test)]

use super::{has_legacy_defs, unpack_legacy_defs};
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
    Detected(bool),
    Object(Object),
    BudgetExceeded(usize),
}

enum Operation {
    HasLegacyDefs(Object),
    UnpackLegacyDefs(Object, usize),
}

fn run(operation: Operation) -> Outcome {
    match operation {
        Operation::HasLegacyDefs(schema) => {
            Outcome::Detected(has_legacy_defs(&Json::Object(schema)))
        }
        Operation::UnpackLegacyDefs(schema, max_inlined_bytes) => {
            match unpack_legacy_defs(schema, max_inlined_bytes) {
                Ok(result) => Outcome::Object(result),
                Err(DefinitionsError::ExpansionBudgetExceeded { limit }) => {
                    Outcome::BudgetExceeded(limit)
                }
            }
        }
    }
}

#[rstest]
#[case::has_legacy_defs_true_for_a_definitions_key(
    Operation::HasLegacyDefs(object(r#"{"definitions": {}}"#)),
    Outcome::Detected(true)
)]
#[case::has_legacy_defs_true_for_components_schemas(
    Operation::HasLegacyDefs(object(r#"{"components": {"schemas": {}}}"#)),
    Outcome::Detected(true)
)]
#[case::has_legacy_defs_false_without_either(
    Operation::HasLegacyDefs(object(r#"{"type": "object"}"#)),
    Outcome::Detected(false)
)]
#[case::has_legacy_defs_false_for_a_non_object_components_value(
    Operation::HasLegacyDefs(object(r#"{"components": "not an object"}"#)),
    Outcome::Detected(false)
)]
#[case::leaves_a_schema_without_legacy_defs_unchanged(
    Operation::UnpackLegacyDefs(object(r#"{"type": "object"}"#), 1_000),
    Outcome::Object(object(r#"{"type": "object"}"#))
)]
#[case::inlines_a_ref_backed_by_definitions(
    Operation::UnpackLegacyDefs(
        object(r##"{"definitions": {"Foo": {"type": "string"}}, "a": {"$ref": "#/$defs/Foo"}}"##),
        1_000
    ),
    Outcome::Object(object(r#"{"a": {"type": "string"}}"#))
)]
#[case::inlines_a_ref_backed_by_components_schemas(
    Operation::UnpackLegacyDefs(
        object(
            r##"{"components": {"schemas": {"Foo": {"type": "string"}}}, "a": {"$ref": "#/$defs/Foo"}}"##
        ),
        1_000
    ),
    Outcome::Object(object(r#"{"a": {"type": "string"}}"#))
)]
#[case::definitions_wins_over_components_schemas_on_a_name_collision(
    Operation::UnpackLegacyDefs(
        object(
            r##"{"components": {"schemas": {"Foo": {"type": "number"}}}, "definitions": {"Foo": {"type": "string"}}, "a": {"$ref": "#/$defs/Foo"}}"##
        ),
        1_000
    ),
    Outcome::Object(object(r#"{"a": {"type": "string"}}"#))
)]
#[case::ignores_a_non_object_definitions_value(
    Operation::UnpackLegacyDefs(
        object(r##"{"definitions": "not an object", "a": {"$ref": "#/$defs/Foo"}}"##),
        1_000
    ),
    Outcome::Object(object(r##"{"a": {"$ref": "#/$defs/Foo"}}"##))
)]
#[case::ignores_a_non_object_components_schemas_value(
    Operation::UnpackLegacyDefs(
        object(
            r##"{"components": {"schemas": "not an object"}, "definitions": {}, "a": {"$ref": "#/$defs/Foo"}}"##
        ),
        1_000
    ),
    Outcome::Object(object(r##"{"a": {"$ref": "#/$defs/Foo"}}"##))
)]
#[case::the_byte_budget_rejects_an_oversized_target(
    Operation::UnpackLegacyDefs(
        object(&format!(
            r##"{{"definitions": {{"Big": {{"description": "{}"}}}}, "a": {{"$ref": "#/$defs/Big"}}}}"##,
            "x".repeat(50)
        )),
        10
    ),
    Outcome::BudgetExceeded(10)
)]
fn schema_definitions_behavior_tests_behavior(
    #[case] operation: Operation,
    #[case] expected: Outcome,
) {
    assert_eq!(run(operation), expected);
}
