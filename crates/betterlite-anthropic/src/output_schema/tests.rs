#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::mapping::{ResponseFormat, json_tool, map_output, map_tool};
use super::{SchemaError, filter_anthropic_output_schema};

#[rstest]
#[case::constraint_description("filter", r#"{"description":"x","minItems":2,"uniqueItems":true,"type":"array"}"#, r#"{"description":"x Note: minimum number of items: 2, all array items must be unique.","type":"array"}"#)]
#[case::false_constraint_ignored("filter", r#"{"uniqueItems":false,"minimum":0,"maximum":null,"minLength":"","contains":[],"not":{}}"#, r#"{"description":"Note: array must contain an item matching: [], minimum value: 0, maximum value: None, minimum length: , must not match: {}."}"#)]
#[case::enum_type_conflict("filter", r#"{"type":"object","enum":[1]}"#, r#"{"enum":[1]}"#)]
#[case::oneof_to_anyof(
    "filter",
    r#"{"oneOf":[{"type":"string"}],"anyOf":[{"type":"number"},{"type":"string"}]}"#,
    r#"{"anyOf":[{"type":"string"},{"type":"number"},{"type":"string"}]}"#
)]
#[case::object_additional_properties(
    "filter",
    r#"{"type":"object","additionalProperties":true}"#,
    r#"{"type":"object","additionalProperties":false}"#
)]
#[case::bounded_def_expansion("output", r##"{"response_schema":{"$defs":{"Name":{"type":"string","minLength":2}},"definitions":{"Name":{"type":"number"}},"properties":{"name":{"$ref":"#/$defs/Name"}}}}"##, r#"{"type":"json_schema","schema":{"properties":{"name":{"type":"string","description":"Note: minimum length: 2."}}}}"#)]
#[case::json_tool_fallback(
    "tool",
    r#"{"type":"string","minLength":2,"$defs":{}}"#,
    r#"{"name":"json_tool_call","input_schema":{"type":"string","minLength":2,"$defs":{}}}"#
)]
#[case::text_format_no_tool("map_tool", r#"{"type":"text","response_schema":{}}"#, "null")]
fn output_schema_cases(#[case] action: &str, #[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        evaluate(action, source),
        serde_json::from_str::<Json>(expected).unwrap()
    );
}

fn evaluate(action: &str, source: &str) -> Json {
    let result = match action {
        "filter" => serde_json::to_value(
            filter_anthropic_output_schema(serde_json::from_str(source).unwrap()).unwrap(),
        )
        .unwrap(),
        "output" => serde_json::to_value(
            map_output(
                serde_json::from_str::<Option<ResponseFormat>>(source)
                    .unwrap()
                    .as_ref(),
            )
            .unwrap(),
        )
        .unwrap(),
        "map_tool" => serde_json::to_value(
            map_tool(
                serde_json::from_str::<Option<ResponseFormat>>(source)
                    .unwrap()
                    .as_ref(),
            )
            .unwrap(),
        )
        .unwrap(),
        "tool" => serde_json::to_value(
            json_tool(
                serde_json::from_str::<Option<Json>>(source)
                    .unwrap()
                    .as_ref(),
            )
            .unwrap(),
        )
        .unwrap(),
        _ => panic!("unknown test action"),
    };
    serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap()
}

#[rstest]
#[case::empty_scalar(r#"{"type":"array","enum":[]}"#, true)]
#[case::list_type(r#"{"type":["string"],"enum":["x"]}"#, false)]
#[case::empty_list_type(r#"{"type":[],"enum":[]}"#, false)]
#[case::unknown_empty(r#"{"type":"unknown","enum":[]}"#, true)]
#[case::null_match(r#"{"type":"null","enum":[null]}"#, true)]
#[case::null_conflict(r#"{"type":"null","enum":[1]}"#, false)]
#[case::boolean_match(r#"{"type":"boolean","enum":[true]}"#, true)]
#[case::boolean_conflict(r#"{"type":"boolean","enum":[1]}"#, false)]
#[case::integer_boolean(r#"{"type":"integer","enum":[true]}"#, false)]
#[case::integer_float(r#"{"type":"integer","enum":[1.5]}"#, false)]
#[case::large_integer(r#"{"type":"integer","enum":[18446744073709551616]}"#, true)]
#[case::integer_match(r#"{"type":"integer","enum":[1]}"#, true)]
#[case::number_match(r#"{"type":"number","enum":[1.5]}"#, true)]
#[case::number_boolean(r#"{"type":"number","enum":[true]}"#, false)]
#[case::number_text(r#"{"type":"number","enum":["text"]}"#, false)]
#[case::array_match(r#"{"type":"array","enum":[[]]}"#, true)]
#[case::array_conflict(r#"{"type":"array","enum":[1]}"#, false)]
#[case::object_match(r#"{"type":"object","enum":[{}]}"#, true)]
#[case::object_conflict(r#"{"type":"object","enum":[1]}"#, false)]
#[case::string_match(r#"{"type":"string","enum":["x"]}"#, true)]
#[case::string_conflict(r#"{"type":"string","enum":[1]}"#, false)]
#[case::nonlist_enum(r#"{"type":"string","enum":null}"#, true)]
#[case::unknown_type(r#"{"type":"unknown","enum":[1]}"#, true)]
#[case::no_type(r#"{"enum":[1]}"#, false)]
#[case::null_type(r#"{"type":null,"enum":[1]}"#, true)]
fn enum_type_regressions(#[case] source: &str, #[case] keeps_type: bool) {
    let result = evaluate("filter", source);
    assert_eq!(
        result.as_object().unwrap().get("type").is_some(),
        keeps_type
    );
    assert_eq!(
        result.as_object().unwrap().get("enum"),
        serde_json::from_str::<Json>(source)
            .unwrap()
            .as_object()
            .unwrap()
            .get("enum")
    );
}

#[rstest]
#[case::boolean(r#"{"minimum":true}"#, "Note: minimum value: True.")]
#[case::unicode(
    r#"{"contains":{"enum":["\u00e9","\ud83d\ude00"]}}"#,
    r#"Note: array must contain an item matching: {"enum": ["\u00e9", "\ud83d\ude00"]}."#
)]
#[case::nested(
    r#"{"contains":{"type":"string","enum":["x"]}}"#,
    r#"Note: array must contain an item matching: {"type": "string", "enum": ["x"]}."#
)]
#[case::float(
    r#"{"minimum":1e-8,"maximum":1e20}"#,
    "Note: minimum value: 1e-08, maximum value: 1e+20."
)]
#[case::empty_description(r#"{"description":false,"minimum":1}"#, "Note: minimum value: 1.")]
fn description_regressions(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        evaluate("filter", source)
            .as_object()
            .unwrap()
            .get("description"),
        Some(&Json::Text(expected.to_owned()))
    );
}

#[test]
fn output_schema_filter_preserves_nested_branches() {
    let source = r#"{"properties":{"child":{"minItems":1}},"$defs":{"D":{"type":"object"}},"items":{"minimum":1},"allOf":[{"minimum":2},false],"extra":{"minimum":3}}"#;
    let expected = r#"{"properties":{"child":{"description":"Note: minimum number of items: 1."}},"$defs":{"D":{"type":"object","additionalProperties":false}},"items":{"description":"Note: minimum value: 1."},"allOf":[{"description":"Note: minimum value: 2."},false],"extra":{"minimum":3}}"#;
    assert_eq!(
        evaluate("filter", source),
        serde_json::from_str::<Json>(expected).unwrap()
    );
}

#[rstest]
#[case::null("null")]
#[case::array("[1,2]")]
#[case::boolean("false")]
#[case::wrong_shapes(r#"{"items":[{"minimum":1}],"properties":[],"$defs":false,"oneOf":{},"anyOf":false,"allOf":null}"#)]
fn preserves_unrecognized_shapes(#[case] source: &str) {
    assert_eq!(
        evaluate("filter", source),
        serde_json::from_str::<Json>(source).unwrap()
    );
}

#[rstest]
#[case::empty(r#"{"response_schema":{}}"#, r#"{"type":"json_schema","schema":{}}"#)]
#[case::nested(
    r#"{"json_schema":{"schema":{"type":"string"}}}"#,
    r#"{"type":"json_schema","schema":{"type":"string"}}"#
)]
#[case::null_precedence(r#"{"response_schema":null,"json_schema":{"schema":{}}}"#, "null")]
#[case::empty_defs(
    r#"{"response_schema":{"$defs":{},"definitions":{"X":{"type":"string"}},"$ref":"X"}}"#,
    r#"{"type":"json_schema","schema":{"$ref":"X"}}"#
)]
#[case::legacy(
    r#"{"response_schema":{"definitions":{"X":{"type":"string"}},"$ref":"X"}}"#,
    r#"{"type":"json_schema","schema":{"type":"string"}}"#
)]
#[case::text_output(
    r#"{"type":"text","response_schema":{}}"#,
    r#"{"type":"json_schema","schema":{}}"#
)]
#[case::absent("null", "null")]
fn extracts_response_schema(#[case] source: &str, #[case] expected: &str) {
    let value: Option<ResponseFormat> = serde_json::from_str(source).unwrap();
    let original = value.clone();
    assert_eq!(
        serde_json::from_value::<Json>(
            serde_json::to_value(map_output(value.as_ref()).unwrap()).unwrap()
        )
        .unwrap(),
        serde_json::from_str::<Json>(expected).unwrap()
    );
    assert_eq!(value, original);
}

#[rstest]
#[case::none("null")]
#[case::text(r#"{"type":"text","response_schema":{}}"#)]
#[case::missing_schema(r#"{"type":"json_object"}"#)]
#[case::null_schema(r#"{"type":"json_schema","response_schema":null}"#)]
fn returns_no_tool(#[case] source: &str) {
    assert_eq!(evaluate("map_tool", source), Json::Null);
}

#[test]
fn tool_requires_type_and_preserves_schema() {
    let missing: ResponseFormat = serde_json::from_str("{}").unwrap();
    assert!(matches!(
        map_tool(Some(&missing)),
        Err(SchemaError::MissingType)
    ));
    assert_eq!(
        evaluate("map_tool", r#"{"type":"other","response_schema":{}}"#),
        evaluate("tool", "{}")
    );
    assert_eq!(
        evaluate("tool", "null"),
        serde_json::from_str::<Json>(r#"{"name":"json_tool_call","input_schema":{"type":"object","additionalProperties":true,"properties":{}}}"#).unwrap()
    );
}

#[rstest]
#[case::description(r#"{"description":true,"minimum":1}"#)]
#[case::alternatives(r#"{"anyOf":false,"oneOf":[]}"#)]
#[case::enum_type(r#"{"type":{},"enum":[]}"#)]
fn rejects_invalid_schema_shapes(#[case] source: &str) {
    assert!(matches!(
        filter_anthropic_output_schema(serde_json::from_str(source).unwrap()),
        Err(SchemaError::Invalid(_))
    ));
}

#[test]
fn projects_all_constraint_labels_in_declared_order() {
    let input = r#"{"not":{},"else":null,"then":[],"if":{},"unevaluatedProperties":true,"dependentSchemas":{},"dependentRequired":{},"propertyNames":{},"patternProperties":{},"maxProperties":2,"minProperties":1,"maxLength":3,"minLength":1,"multipleOf":2,"exclusiveMaximum":9,"exclusiveMinimum":0,"maximum":10,"minimum":1,"prefixItems":[],"maxContains":2,"minContains":1,"contains":{},"uniqueItems":true,"maxItems":3,"minItems":1}"#;
    let expected = "Note: minimum number of items: 1, maximum number of items: 3, all array items must be unique, array must contain an item matching: {}, minimum number of matching items: 1, maximum number of matching items: 2, leading items must match, in order: [], minimum value: 1, maximum value: 10, exclusive minimum value: 0, exclusive maximum value: 9, must be a multiple of 2, minimum length: 1, maximum length: 3, minimum number of properties: 1, maximum number of properties: 2, properties whose names match each pattern must satisfy: {}, property names must satisfy: {}, dependent required properties: {}, dependent schemas: {}, unevaluated properties must satisfy: True, conditional (if): {}, conditional (then): [], conditional (else): None, must not match: {}.";
    assert_eq!(
        evaluate("filter", input),
        Json::Object(Object::from_iter([(
            String::from("description"),
            Json::Text(expected.to_owned())
        )]))
    );
}

#[path = "limits/tests.rs"]
mod limits;
