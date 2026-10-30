#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{ResponseFormat, filter_anthropic_output_schema, map_output};

#[rstest]
#[case::constraint_description(
    r#"{"description":"x","minItems":2,"uniqueItems":true,"type":"array"}"#,
    "Note: minimum number of items: 2, all array items must be unique."
)]
#[case::false_constraint_ignored(r#"{"type":"array","uniqueItems":false}"#, "")]
#[case::enum_type_conflict(r#"{"type":"string","enum":[1]}"#, "enum")]
#[case::oneof_to_anyof(r#"{"oneOf":[{"type":"string"}]}"#, "anyOf")]
#[case::object_additional_properties(
    r#"{"type":"object","additionalProperties":true}"#,
    "additionalProperties"
)]
#[case::bounded_def_expansion(r##"{"$defs":{"Name":{"type":"string"}},"type":"object","properties":{"name":{"$ref":"#/$defs/Name"}}}"##, "properties")]
#[case::json_tool_fallback(r#"{"type":"string"}"#, "tool")]
#[case::text_format_no_tool("null", "none")]
fn output_schema_cases(#[case] source: &str, #[case] expected: &str) {
    let schema: Json = serde_json::from_str(source).unwrap();
    if expected == "tool" {
        let tool = super::map_tool(Some(ResponseFormat::JsonSchema { schema })).unwrap();
        assert_eq!(tool.name, "json_tool_call");
    } else if expected == "none" {
        assert!(map_output(None).is_none());
    } else {
        let result = if expected == "properties" {
            map_output(Some(ResponseFormat::JsonSchema { schema }))
                .unwrap()
                .schema
        } else {
            filter_anthropic_output_schema(schema)
        };
        let filtered = serde_json::to_value(result).unwrap();
        if expected == "Note: minimum number of items: 2, all array items must be unique." {
            assert_eq!(
                filtered["description"],
                "x Note: minimum number of items: 2, all array items must be unique."
            );
        }
        if expected == "enum" {
            assert!(filtered.get("type").is_none());
            for conflict_source in [
                r#"{"type":"array","enum":[]}"#,
                r#"{"type":"object","enum":[1]}"#,
                r#"{"type":"integer","enum":[1.5]}"#,
                r#"{"type":"number","enum":["text"]}"#,
            ] {
                let conflict_schema: Json = serde_json::from_str(conflict_source).unwrap();
                let conflict_result =
                    serde_json::to_value(filter_anthropic_output_schema(conflict_schema)).unwrap();
                assert!(conflict_result.get("type").is_none());
            }
            for matching_source in [
                r#"{"type":"array","enum":[[]]}"#,
                r#"{"type":"object","enum":[{}]}"#,
            ] {
                let matching_schema: Json = serde_json::from_str(matching_source).unwrap();
                let matching_result =
                    serde_json::to_value(filter_anthropic_output_schema(matching_schema)).unwrap();
                assert!(matching_result.get("type").is_some());
            }
        }
        if expected == "anyOf" {
            assert!(filtered.get("anyOf").is_some());
            let ordered_schema: Json = serde_json::from_str(
                r#"{"oneOf":[{"type":"string"}],"anyOf":[{"type":"number"}]}"#,
            )
            .unwrap();
            let ordered_result =
                serde_json::to_value(filter_anthropic_output_schema(ordered_schema)).unwrap();
            assert_eq!(ordered_result["anyOf"][0]["type"], "string");
            assert_eq!(ordered_result["anyOf"][1]["type"], "number");
        }
        if expected == "additionalProperties" {
            assert_eq!(filtered["additionalProperties"], false);
        }
        if expected == "properties" {
            assert_eq!(filtered["properties"]["name"]["type"], "string");
        }
    }
}

#[test]
fn output_schema_filter_preserves_each_supported_branch() {
    let source: Json = serde_json::from_str(
        r#"{
            "description":"schema",
            "type":"string",
            "contains":{"type":"object","minimum":1},
            "properties":{"child":{"minItems":1,"type":"array"}},
            "$defs":{"Definition":{"type":"boolean"}},
            "items":{"type":"object","additionalProperties":true}
        }"#,
    )
    .unwrap();
    let filtered = serde_json::to_value(filter_anthropic_output_schema(source)).unwrap();
    assert_eq!(filtered["type"], "string");
    assert_eq!(
        filtered["description"],
        "schema Note: array must contain an item matching: {\"type\": \"object\", \"minimum\": 1}."
    );
    assert_eq!(
        filtered["properties"]["child"]["description"],
        "Note: minimum number of items: 1."
    );
    assert_eq!(filtered["$defs"]["Definition"]["type"], "boolean");
    assert_eq!(filtered["items"]["type"], "object");
    assert_eq!(filtered["items"]["additionalProperties"], false);
    assert_eq!(
        serde_json::to_value(filter_anthropic_output_schema(
            serde_json::from_str(r#"{"type":"null","enum":[null]}"#).unwrap(),
        ))
        .unwrap()["type"],
        "null"
    );
    assert_eq!(
        serde_json::to_value(filter_anthropic_output_schema(
            serde_json::from_str(r#"{"type":"boolean","enum":[true]}"#).unwrap(),
        ))
        .unwrap()["type"],
        "boolean"
    );
    assert_eq!(
        serde_json::to_value(filter_anthropic_output_schema(
            serde_json::from_str(r#"{"type":"array","enum":[[]]}"#).unwrap(),
        ))
        .unwrap()["type"],
        "array"
    );
    let boolean_conflict: Json = serde_json::from_str(r#"{"type":"boolean","enum":[1]}"#).unwrap();
    let boolean_result =
        serde_json::to_value(filter_anthropic_output_schema(boolean_conflict)).unwrap();
    assert!(boolean_result.get("type").is_none());
    let array_conflict: Json = serde_json::from_str(r#"{"type":"array","enum":[1]}"#).unwrap();
    let array_result =
        serde_json::to_value(filter_anthropic_output_schema(array_conflict)).unwrap();
    assert!(array_result.get("type").is_none());
    let note_schema: Json =
        serde_json::from_str(r#"{"contains":{"type":"string","enum":["x"]}}"#).unwrap();
    let note = serde_json::to_value(filter_anthropic_output_schema(note_schema)).unwrap();
    assert_eq!(
        note["description"],
        "Note: array must contain an item matching: {\"type\": \"string\", \"enum\": [\"x\"]}."
    );
}
