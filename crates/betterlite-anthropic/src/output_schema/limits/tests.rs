#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use crate::output_schema::mapping::{Field, ResponseFormat, map_output};
use crate::output_schema::{SchemaError, filter_anthropic_output_schema};

#[test]
fn rejects_excessive_schema_depth() {
    let schema = (0_u16..130).fold(Json::Null, |child, _| {
        Json::Object(Object::from_iter([(String::from("items"), child)]))
    });
    assert!(matches!(
        filter_anthropic_output_schema(schema),
        Err(SchemaError::Limit)
    ));
}

#[test]
fn propagates_definition_expansion_budget() {
    let format = ResponseFormat {
        type_name: Field::Missing,
        json_schema: Field::Missing,
        response_schema: Field::Value(
            serde_json::from_str(&format!(r#"{{"$defs":{{"Big":{{"description":"{}"}}}},"properties":{{"one":{{"$ref":"Big"}},"two":{{"$ref":"Big"}}}}}}"#, "x".repeat(5_000_000)))
            .unwrap(),
        ),
    };
    assert!(matches!(
        map_output(Some(&format)),
        Err(SchemaError::Definitions(_))
    ));
}

#[test]
fn rejects_excessive_schema_bytes() {
    assert!(matches!(
        filter_anthropic_output_schema(Json::Text("x".repeat(10_000_001))),
        Err(SchemaError::Limit)
    ));
}

#[rstest]
#[case::object_limit(128, "object", true)]
#[case::array_limit(128, "array", true)]
#[case::array_excess(129, "array", false)]
fn enforces_depth_boundary(#[case] depth: usize, #[case] kind: &str, #[case] accepted: bool) {
    let mut schema = Json::Null;
    for _ in 0..depth {
        schema = if kind == "array" {
            Json::List(vec![schema])
        } else {
            Json::Object(Object::from_iter([(String::from("items"), schema)]))
        };
    }
    assert_eq!(filter_anthropic_output_schema(schema).is_ok(), accepted);
}

#[test]
fn accepts_schema_at_byte_limit() {
    let schema = Json::Text("x".repeat(9_999_998));
    assert_eq!(
        filter_anthropic_output_schema(schema.clone()).unwrap(),
        schema
    );
}
