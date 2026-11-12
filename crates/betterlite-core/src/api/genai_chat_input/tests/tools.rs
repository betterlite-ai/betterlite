use rstest::rstest;
use serde_json::{Value, json};

use super::prelude::{limits_with_nodes, translate, translate_result};
use crate::api::genai_chat_input::GenAiInputError;

#[test]
fn function_declarations_become_openai_tools() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"functionDeclarations": [
            {"name": "lookup", "description": "find one", "parameters": {"type": "object"}},
        ]}],
    }));
    let tools = request.tools.as_ref().expect("tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].kind, "function");
    let function = tools[0].function.as_ref().expect("function");
    assert_eq!(function.name, "lookup");
    assert_eq!(function.description.as_deref(), Some("find one"));
    assert_eq!(function.parameters, Some(json!({"type": "object"})));
}

#[test]
fn every_declaration_of_every_tool_is_kept_in_order() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [
            {"functionDeclarations": [{"name": "a"}, {"name": "b"}]},
            {"functionDeclarations": [{"name": "c"}]},
            {"otherTool": {}},
        ],
    }));
    let names: Vec<&str> = request
        .tools
        .as_ref()
        .expect("tools")
        .iter()
        .filter_map(|tool| {
            tool.function
                .as_ref()
                .map(|function| function.name.as_str())
        })
        .collect();
    assert_eq!(names, ["a", "b", "c"]);
}

#[test]
fn parameters_json_schema_wins_over_parameters() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"functionDeclarations": [{
            "name": "lookup",
            "parameters": {"type": "object", "first": true},
            "parametersJsonSchema": {"type": "object", "second": true},
        }]}],
    }));
    let parameters = request.tools.as_ref().expect("tools")[0]
        .function
        .as_ref()
        .expect("function")
        .parameters
        .clone();
    assert_eq!(parameters, Some(json!({"type": "object", "second": true})));
}

#[test]
fn a_null_parameters_json_schema_falls_back_to_parameters() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"functionDeclarations": [{
            "name": "lookup",
            "parameters": {"type": "object", "kept": true},
            "parametersJsonSchema": null,
        }]}],
    }));
    let parameters = request.tools.as_ref().expect("tools")[0]
        .function
        .as_ref()
        .expect("function")
        .parameters
        .clone();
    assert_eq!(parameters, Some(json!({"type": "object", "kept": true})));
}

#[rstest]
#[case::string(json!("schema"))]
#[case::array(json!([{"type": "object"}]))]
#[case::number(json!(7_i32))]
#[case::boolean(json!(true))]
fn a_non_object_parameter_schema_is_dropped(#[case] parameters: Value) {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"functionDeclarations": [{"name": "lookup", "parameters": parameters}]}],
    }));
    let function = request.tools.as_ref().expect("tools")[0]
        .function
        .as_ref()
        .expect("function");
    assert_eq!(function.parameters, None);
}

#[test]
fn the_declaration_schema_types_are_lowercased() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"functionDeclarations": [{
            "name": "lookup",
            "parameters": {
                "type": "OBJECT",
                "properties": {"size": {"type": "INTEGER"}},
                "items": {"type": "STRING"},
            },
        }]}],
    }));
    let parameters = request.tools.as_ref().expect("tools")[0]
        .function
        .as_ref()
        .expect("function")
        .parameters
        .clone()
        .expect("parameters");
    assert_eq!(parameters["type"], "object");
    assert_eq!(parameters["properties"]["size"]["type"], "integer");
    assert_eq!(parameters["items"]["type"], "string");
}

#[test]
fn a_declaration_without_a_name_keeps_an_empty_name() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"functionDeclarations": [{"description": "unnamed"}]}],
    }));
    let function = request.tools.as_ref().expect("tools")[0]
        .function
        .as_ref()
        .expect("function");
    assert_eq!(function.name, "");
}

#[test]
fn tools_without_declarations_do_not_add_a_tools_field() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"otherTool": {}}],
    }));
    assert_eq!(request.tools, None);
}

#[test]
fn an_empty_tools_list_does_not_add_a_tools_field() {
    let request = translate(json!({"model": "m", "contents": [], "tools": []}));
    assert_eq!(request.tools, None);
}

#[test]
fn a_null_tools_list_does_not_add_a_tools_field() {
    let request = translate(json!({"model": "m", "contents": [], "tools": null}));
    assert_eq!(request.tools, None);
}

#[test]
fn null_function_declarations_do_not_add_a_tool() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tools": [{"functionDeclarations": null}],
    }));
    assert_eq!(request.tools, None);
}

#[test]
fn tool_parameters_past_the_limits_fail_the_translation() {
    let limits = limits_with_nodes(3);
    let error = translate_result(
        json!({
            "model": "m",
            "contents": [],
            "tools": [{"functionDeclarations": [{
                "name": "lookup",
                "parameters": {
                    "type": "object",
                    "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
                },
            }]}],
        }),
        None,
        limits,
    )
    .expect_err("a schema limit error");
    assert!(matches!(error, GenAiInputError::Schema(_)));
}
