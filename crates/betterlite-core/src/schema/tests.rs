#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        clippy::default_numeric_fallback,
        reason = "json! fixtures state the schema documents the normalizer and validator see"
    )
)]

use std::fs;
use std::path::PathBuf;

use betterlite_error::error::ErrorClass;
use rstest::rstest;

use super::bounds::{SchemaCheck, check_schema};
use super::normalize::{normalize_json_schema_types, normalize_tool_schema};
use super::validate::validate_schema;
use super::{SchemaError, SchemaJson, SchemaLimits};

const MAX_CORPUS_BYTES: usize = 0x0004_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Normalize,
    Tool,
    Validate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    None,
    InvalidJson,
    Mismatch,
    InvalidSchema,
    ExternalReference,
    SchemaTooLarge,
    TooManyNodes,
    SchemaTooDeep,
    ResponseTooLarge,
}

impl Failure {
    fn of(error: &SchemaError) -> Self {
        match error {
            SchemaError::InvalidJson { .. } => Self::InvalidJson,
            SchemaError::Mismatch { .. } => Self::Mismatch,
            SchemaError::InvalidSchema { .. } => Self::InvalidSchema,
            SchemaError::ExternalReference { .. } => Self::ExternalReference,
            SchemaError::SchemaTooLarge { .. } => Self::SchemaTooLarge,
            SchemaError::TooManyNodes { .. } => Self::TooManyNodes,
            SchemaError::SchemaTooDeep { .. } => Self::SchemaTooDeep,
            SchemaError::ResponseTooLarge { .. } => Self::ResponseTooLarge,
        }
    }
}

struct Case {
    operation: Operation,
    schema: SchemaJson,
    response: &'static str,
    limits: Option<SchemaLimits>,
    expected: Option<SchemaJson>,
    failure: Failure,
}

#[rstest]
#[case::uppercase_types_are_lowercased(Case {
    operation: Operation::Normalize,
    schema: serde_json::json!({
        "type": "OBJECT",
        "properties": {
            "name": {"type": "STRING"},
            "tags": {"type": "ARRAY", "items": {"type": "INTEGER"}},
            "flag": {"type": "BOOLEAN"},
            "score": {"type": "NUMBER"},
            "extra": {"type": "NULL"},
            "choice": {"type": "FUNDAMENTAL"}
        }
    }),
    response: "",
    limits: None,
    expected: Some(serde_json::json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "tags": {"type": "array", "items": {"type": "integer"}},
            "flag": {"type": "boolean"},
            "score": {"type": "number"},
            "extra": {"type": "null"},
            "choice": {"type": "FUNDAMENTAL"}
        }
    })),
    failure: Failure::None,
})]
#[case::lowercase_types_are_kept(Case {
    operation: Operation::Normalize,
    schema: serde_json::json!({"type": "string"}),
    response: "",
    limits: None,
    expected: Some(serde_json::json!({"type": "string"})),
    failure: Failure::None,
})]
#[case::non_string_types_are_kept(Case {
    operation: Operation::Normalize,
    schema: serde_json::json!({"type": ["string", "null"]}),
    response: "",
    limits: None,
    expected: Some(serde_json::json!({"type": ["string", "null"]})),
    failure: Failure::None,
})]
#[case::other_nested_values_are_normalized(Case {
    operation: Operation::Normalize,
    schema: serde_json::json!({"anyOf": [{"type": "INTEGER"}], "additionalProperties": {"type": "STRING"}}),
    response: "",
    limits: None,
    expected: Some(serde_json::json!({"anyOf": [{"type": "integer"}], "additionalProperties": {"type": "string"}})),
    failure: Failure::None,
})]
#[case::normalization_stops_at_the_depth_limit(Case {
    operation: Operation::Normalize,
    schema: serde_json::json!({
        "properties": {
            "a": {"type": "STRING", "properties": {"b": {"type": "NUMBER"}}}
        }
    }),
    response: "",
    limits: Some(SchemaLimits { max_depth: 2, ..SchemaLimits::default() }),
    expected: Some(serde_json::json!({
        "properties": {
            "a": {"type": "string", "properties": {"b": {"type": "NUMBER"}}}
        }
    })),
    failure: Failure::None,
})]
#[case::tool_parameters_are_normalized(Case {
    operation: Operation::Tool,
    schema: serde_json::json!({
        "type": "function",
        "function": {"name": "f", "parameters": {"type": "OBJECT", "properties": {"x": {"type": "STRING"}}}}
    }),
    response: "",
    limits: None,
    expected: Some(serde_json::json!({
        "type": "function",
        "function": {"name": "f", "parameters": {"type": "object", "properties": {"x": {"type": "string"}}}}
    })),
    failure: Failure::None,
})]
#[case::tool_without_parameters_is_kept(Case {
    operation: Operation::Tool,
    schema: serde_json::json!({"type": "function", "function": {"name": "f"}}),
    response: "",
    limits: None,
    expected: Some(serde_json::json!({"type": "function", "function": {"name": "f"}})),
    failure: Failure::None,
})]
#[case::non_object_function_is_kept(Case {
    operation: Operation::Tool,
    schema: serde_json::json!({"type": "function", "function": "nope"}),
    response: "",
    limits: None,
    expected: Some(serde_json::json!({"type": "function", "function": "nope"})),
    failure: Failure::None,
})]
#[case::non_object_tool_is_kept(Case {
    operation: Operation::Tool,
    schema: serde_json::json!(true),
    response: "",
    limits: None,
    expected: Some(serde_json::json!(true)),
    failure: Failure::None,
})]
#[case::valid_output_matches(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}),
    response: "{\"name\": \"Ada\"}",
    limits: None,
    expected: None,
    failure: Failure::None,
})]
#[case::invalid_json_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object"}),
    response: "this is not json",
    limits: None,
    expected: None,
    failure: Failure::InvalidJson,
})]
#[case::schema_mismatch_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object", "required": ["name"]}),
    response: "{\"age\": 36}",
    limits: None,
    expected: None,
    failure: Failure::Mismatch,
})]
#[case::invalid_schema_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": 42}),
    response: "{}",
    limits: None,
    expected: None,
    failure: Failure::InvalidSchema,
})]
#[case::external_https_reference_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"$ref": "https://example.com/schema.json"}),
    response: "{}",
    limits: None,
    expected: None,
    failure: Failure::ExternalReference,
})]
#[case::external_file_reference_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"$ref": "file:///etc/passwd"}),
    response: "{}",
    limits: None,
    expected: None,
    failure: Failure::ExternalReference,
})]
#[case::relative_reference_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"$ref": "other.json#/definitions/x"}),
    response: "{}",
    limits: None,
    expected: None,
    failure: Failure::ExternalReference,
})]
#[case::internal_reference_validates(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"properties": {"a": {"$ref": "#/$defs/n"}}, "$defs": {"n": {"type": "integer"}}}),
    response: "{\"a\": 1}",
    limits: None,
    expected: None,
    failure: Failure::None,
})]
#[case::schema_too_large_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object"}),
    response: "{}",
    limits: Some(SchemaLimits { max_schema_bytes: 4, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::SchemaTooLarge,
})]
#[case::schema_at_the_byte_limit_passes(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object"}),
    response: "{}",
    limits: Some(SchemaLimits { max_schema_bytes: 16, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::None,
})]
#[case::too_many_nodes_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object"}),
    response: "{}",
    limits: Some(SchemaLimits { max_nodes: 1, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::TooManyNodes,
})]
#[case::nodes_at_the_limit_pass(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object"}),
    response: "{}",
    limits: Some(SchemaLimits { max_nodes: 2, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::None,
})]
#[case::schema_too_deep_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"properties": {"a": true}}),
    response: "{}",
    limits: Some(SchemaLimits { max_depth: 1, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::SchemaTooDeep,
})]
#[case::depth_at_the_limit_passes(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"properties": {"a": true}}),
    response: "{}",
    limits: Some(SchemaLimits { max_depth: 2, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::None,
})]
#[case::response_too_large_fails(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object"}),
    response: "{\"a\": 1}",
    limits: Some(SchemaLimits { max_response_bytes: 3, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::ResponseTooLarge,
})]
#[case::response_at_the_limit_passes(Case {
    operation: Operation::Validate,
    schema: serde_json::json!({"type": "object"}),
    response: "{}",
    limits: Some(SchemaLimits { max_response_bytes: 2, ..SchemaLimits::default() }),
    expected: None,
    failure: Failure::None,
})]
fn call_rules_tests_behavior(#[case] case: Case) {
    let limits = case.limits.unwrap_or_default();
    let outcome = match case.operation {
        Operation::Normalize => normalize_json_schema_types(&case.schema, &limits).map(Some),
        Operation::Tool => normalize_tool_schema(&case.schema, &limits).map(Some),
        Operation::Validate => validate_schema(&case.schema, case.response, &limits).map(|()| None),
    };
    match (outcome, case.failure) {
        (Ok(value), Failure::None) => assert_eq!(value, case.expected),
        (Err(error), failure) => assert_eq!(Failure::of(&error), failure),
        (Ok(_), failure) => panic!("expected a {failure:?} failure"),
    }
}

#[test]
fn format_keywords_are_annotations() {
    let schema = serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "string",
        "format": "email"
    });
    validate_schema(&schema, "\"not an email\"", &SchemaLimits::default()).unwrap();
}

#[test]
fn a_local_reference_is_kept() {
    let schema = serde_json::json!({"$ref": "", "definitions": {"x": {"type": "string"}}});
    check_schema(&schema, &SchemaLimits::default(), SchemaCheck::Validation).unwrap();
}

#[test]
fn an_external_reference_names_the_document() {
    let schema = serde_json::json!({"$ref": "https://example.com/schema.json"});
    let error = validate_schema(&schema, "{}", &SchemaLimits::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the schema references the external document https://example.com/schema.json"
    );
}

#[test]
fn a_validation_failure_classifies_as_json_schema_validation() {
    let schema = serde_json::json!({"type": "object"});
    let error = validate_schema(&schema, "[]", &SchemaLimits::default()).unwrap_err();
    let class = error.classify(&schema, "[]");
    assert!(matches!(class, ErrorClass::JsonSchemaValidation(_)));
    assert_eq!(class.metadata().status_code, 500);
}

#[test]
fn a_schema_the_validator_cannot_compile_classifies_as_internal_server() {
    let schema = serde_json::json!({"type": 42});
    let error = validate_schema(&schema, "{}", &SchemaLimits::default()).unwrap_err();
    let class = error.classify(&schema, "{}");
    assert!(matches!(class, ErrorClass::InternalServer(_)));
    assert_eq!(class.metadata().status_code, 500);
}

#[rstest]
fn replays_the_response_schema_corpus(#[files("fuzz/corpus/response_schema/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > MAX_CORPUS_BYTES {
        return;
    }
    let Ok(case) = serde_json::from_slice::<SchemaJson>(&bytes) else {
        return;
    };
    let Some(fields) = case.as_object() else {
        return;
    };
    let limits = SchemaLimits::default();
    if let Some(schema) = fields.get("schema") {
        drop(normalize_json_schema_types(schema, &limits));
        if let Some(response) = fields.get("response").and_then(SchemaJson::as_str) {
            drop(validate_schema(schema, response, &limits));
        }
    }
    if let Some(tool) = fields.get("tool") {
        drop(normalize_tool_schema(tool, &limits));
    }
}
