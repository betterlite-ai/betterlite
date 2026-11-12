#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

use super::{Input, Mode, Op};

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_schema"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

#[test]
fn a_schema_seed_selects_the_vertex_schema_operation() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && case.call.operation == "vertex_schema"
    }));
}

#[test]
fn the_seed_parses_into_the_build_variant() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty());
    let inputs: Vec<Input> = seeds
        .iter()
        .map(|source| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            serde_json::from_value(case.call.input).unwrap()
        })
        .collect();
    assert!(inputs.iter().any(|input| {
        matches!(input.mode, Mode::BuildVertexSchema)
            && input.settings.is_empty()
            && input.add_property_ordering
    }));
}

fn run(request: &Value) -> Value {
    let input: Input = serde_json::from_value(request.clone()).unwrap();
    let input_copy = request.clone();
    let output = Op::run(&input).unwrap();
    assert_eq!(serde_json::to_value(&input).unwrap(), input_copy);
    output
}

#[test]
fn the_adapter_returns_shaped_vertex_schemas() {
    let source = r#"{"mode": "build_vertex_schema", "schema": {"type": "object", "properties": {"b": {"type": "string"}, "a": {"type": "integer"}}}, "add_property_ordering": true, "settings": {}}"#;
    let input: Input = serde_json::from_str(source).unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(
        output,
        json!({
            "type": "object",
            "properties": {"b": {"type": "string"}, "a": {"type": "integer"}},
            "propertyOrdering": ["b", "a"],
        })
    );
}

#[test]
fn the_adapter_passes_a_json_schema_through() {
    let document = r##"{"$defs": {"A": {"type": "string"}}, "type": "object", "additionalProperties": false, "properties": {"a": {"$ref": "#/$defs/A"}}}"##;
    assert_eq!(
        run(&json!({
            "mode": "build_json_schema",
            "schema": serde_json::from_str::<Value>(document).unwrap(),
            "add_property_ordering": true,
            "settings": {"DEFAULT_MAX_RECURSE_DEPTH": "0"},
        })),
        serde_json::from_str::<Value>(document).unwrap()
    );
}

#[test]
fn the_adapter_reads_the_depth_limit_through_the_config_section() {
    let nested = r#"{"properties": {"a": {"properties": {"b": {"type": "string"}}}}}"#;
    let request = json!({
        "mode": "build_vertex_schema",
        "schema": serde_json::from_str::<Value>(nested).unwrap(),
        "add_property_ordering": false,
        "settings": {"DEFAULT_MAX_RECURSE_DEPTH": "2"},
    });
    let input: Input = serde_json::from_value(request).unwrap();
    Op::run(&input).expect_err("the configured depth rejects the nested schema");
}

#[test]
fn the_adapter_rejects_a_depth_that_is_not_a_count() {
    let request = json!({
        "mode": "build_vertex_schema",
        "schema": {"type": "object"},
        "add_property_ordering": false,
        "settings": {"DEFAULT_MAX_RECURSE_DEPTH": "many"},
    });
    let input: Input = serde_json::from_value(request).unwrap();
    Op::run(&input).expect_err("a depth that is not a count is a config error");
}
