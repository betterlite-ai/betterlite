#![cfg(test)]

use rstest::rstest;
use serde_json::json;

use super::{ResponseFormat, filter_anthropic_output_schema, map_output};

#[rstest]
#[case::constraint_description(json!({"description":"x", "minItems": 2_i32, "uniqueItems": true, "type":"array"}), "Note: minimum number of items: 2, all array items must be unique.")]
#[case::false_constraint_ignored(json!({"type":"array", "uniqueItems":false}), "")]
#[case::enum_type_conflict(json!({"type":"string", "enum":[1_i32]}), "enum")]
#[case::oneof_to_anyof(json!({"oneOf":[{"type":"string"}]}), "anyOf")]
#[case::object_additional_properties(json!({"type":"object", "additionalProperties":true}), "additionalProperties")]
#[case::bounded_def_expansion(json!({"$defs":{"Name":{"type":"string"}}, "type":"object", "properties":{"name":{"$ref":"#/$defs/Name"}}}), "properties")]
#[case::json_tool_fallback(json!({"type":"string"}), "tool")]
#[case::text_format_no_tool(json!(null), "none")]
fn output_schema_cases(#[case] schema: serde_json::Value, #[case] expected: &str) {
    if expected == "tool" {
        let tool = super::map_tool(Some(ResponseFormat::JsonSchema { schema })).unwrap();
        assert_eq!(tool.name, "json_tool_call");
    } else if expected == "none" {
        assert!(map_output(None).is_none());
    } else {
        let filtered = if expected == "properties" {
            map_output(Some(ResponseFormat::JsonSchema { schema }))
                .unwrap()
                .schema
        } else {
            filter_anthropic_output_schema(schema)
        };
        if expected == "Note: minimum number of items: 2, all array items must be unique." {
            assert_eq!(
                filtered["description"],
                "x Note: minimum number of items: 2, all array items must be unique."
            );
        }
        if expected == "enum" {
            assert!(filtered.get("type").is_none());
        }
        if expected == "anyOf" {
            assert!(filtered.get("anyOf").is_some());
        }
        if expected == "additionalProperties" {
            assert_eq!(filtered["additionalProperties"], false);
        }
        if expected == "properties" {
            assert_eq!(filtered["properties"]["name"]["type"], "string");
        }
    }
}
