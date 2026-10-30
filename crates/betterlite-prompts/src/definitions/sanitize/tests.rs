#![cfg(test)]

use super::{AnthropicInputSchema, sanitize_for_anthropic};
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
    Schema(Box<AnthropicInputSchema>),
    BudgetExceeded(usize),
}

fn run(input_schema: Object) -> Outcome {
    match sanitize_for_anthropic(input_schema) {
        Ok(schema) => Outcome::Schema(Box::new(schema)),
        Err(DefinitionsError::ExpansionBudgetExceeded { limit }) => Outcome::BudgetExceeded(limit),
    }
}

struct SchemaFixture<'text> {
    schema_type: &'text str,
    properties: &'text str,
    additional_properties: Option<&'text str>,
    required: Option<&'text str>,
    defs: Option<&'text str>,
    strict: Option<&'text str>,
}

fn schema(fixture: &SchemaFixture<'_>) -> Outcome {
    Outcome::Schema(Box::new(AnthropicInputSchema {
        schema_type: json(fixture.schema_type),
        properties: json(fixture.properties),
        additional_properties: fixture.additional_properties.map(json),
        required: fixture.required.map(json),
        defs: fixture.defs.map(json),
        strict: fixture.strict.map(json),
    }))
}

#[rstest]
#[case::forces_a_missing_type_to_object(
    object(r#"{"properties": {"a": {"type": "string"}}}"#),
    schema(&SchemaFixture {
        schema_type: "\"object\"",
        properties: r#"{"a": {"type": "string"}}"#,
        additional_properties: None,
        required: None,
        defs: None,
        strict: None,
    })
)]
#[case::forces_a_mismatched_type_to_object(
    object(r#"{"type": "string", "properties": {}}"#),
    schema(&SchemaFixture {
        schema_type: "\"object\"",
        properties: "{}",
        additional_properties: None,
        required: None,
        defs: None,
        strict: None,
    })
)]
#[case::defaults_properties_to_an_empty_object(
    object(r#"{"type": "object"}"#),
    schema(&SchemaFixture {
        schema_type: "\"object\"",
        properties: "{}",
        additional_properties: None,
        required: None,
        defs: None,
        strict: None,
    })
)]
#[case::keeps_additional_properties_required_defs_and_strict(
    object(
        r#"{"type": "object", "properties": {}, "additionalProperties": false, "required": ["a"], "$defs": {"Foo": {"type": "string"}}, "strict": true}"#
    ),
    schema(&SchemaFixture {
        schema_type: "\"object\"",
        properties: "{}",
        additional_properties: Some("false"),
        required: Some(r#"["a"]"#),
        defs: Some(r#"{"Foo": {"type": "string"}}"#),
        strict: Some("true"),
    })
)]
#[case::drops_a_key_outside_the_anthropic_shape(
    object(r#"{"type": "object", "properties": {}, "patternProperties": {"^x-": {}}}"#),
    schema(&SchemaFixture {
        schema_type: "\"object\"",
        properties: "{}",
        additional_properties: None,
        required: None,
        defs: None,
        strict: None,
    })
)]
#[case::inlines_legacy_definitions_before_filtering(
    object(
        r##"{"type": "object", "definitions": {"Foo": {"type": "string"}}, "properties": {"a": {"$ref": "#/$defs/Foo"}}}"##
    ),
    schema(&SchemaFixture {
        schema_type: "\"object\"",
        properties: r#"{"a": {"type": "string"}}"#,
        additional_properties: None,
        required: None,
        defs: None,
        strict: None,
    })
)]
#[case::propagates_the_expansion_budget_error(
    object(&format!(
        r##"{{"type": "object", "definitions": {{"Big": {{"description": "{}"}}}}, "properties": {{"a": {{"$ref": "#/$defs/Big"}}}}}}"##,
        "x".repeat(10_000_000)
    )),
    Outcome::BudgetExceeded(10_000_000)
)]
fn schema_definitions_behavior_tests_behavior(
    #[case] input_schema: Object,
    #[case] expected: Outcome,
) {
    assert_eq!(run(input_schema), expected);
}
