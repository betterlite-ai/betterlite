#![cfg(test)]

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::anyof::convert_anyof_null_to_nullable;
use super::build::{SchemaFormat, shape_parameters};
use super::enum_empty_strings::fix_enum_empty_strings;
use super::enum_types::fix_enum_types;
use super::error::VertexSchemaError;
use super::filter::filter_schema_fields;
use super::items::process_items;
use super::ordering::set_schema_property_ordering;
use super::recursion::Recursion;
use super::type_arrays::convert_schema_types;

const DEFAULT_DEPTH: usize = 100;

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

fn settings(max_recurse_depth: usize) -> Settings {
    Settings { max_recurse_depth }
}

fn shape(text: &str, ordering: bool) -> Object {
    shape_parameters(
        object(text),
        SchemaFormat::ResponseSchema,
        ordering,
        &settings(DEFAULT_DEPTH),
    )
    .expect("the schema shapes")
}

#[track_caller]
fn assert_shaped(text: &str, expected: &str) {
    assert_eq!(shape(text, false), object(expected));
}

#[track_caller]
fn assert_ordered(text: &str, expected: &str) {
    assert_eq!(shape(text, true), object(expected));
}

const JSON_SCHEMA_DOCUMENT: &str = r##"{
    "$defs": {"A": {"type": "object", "properties": {"value": {"type": "string"}}}},
    "type": "object",
    "additionalProperties": false,
    "properties": {"a": {"$ref": "#/$defs/A"}, "b": {"type": "string", "format": "email"}}
}"##;

#[rstest]
#[case::without_ordering(false)]
#[case::with_ordering(true)]
fn the_json_schema_path_returns_the_document_unchanged(#[case] ordering: bool) {
    let original = object(JSON_SCHEMA_DOCUMENT);
    let shaped = shape_parameters(
        original.clone(),
        SchemaFormat::JsonSchema,
        ordering,
        &settings(0),
    )
    .expect("the JSON Schema path never fails");
    assert_eq!(shaped, original);
    assert_eq!(original, object(JSON_SCHEMA_DOCUMENT));
}

#[test]
fn definitions_are_expanded_and_the_root_table_is_removed() {
    assert_shaped(
        r##"{"type": "object", "properties": {"tree": {"$ref": "#/$defs/Node"}},
            "$defs": {"Node": {"type": "object", "properties": {"next": {"$ref": "#/$defs/Node"}, "value": {"type": "string"}}}}}"##,
        r#"{"type": "object", "properties": {"tree": {"type": "object", "properties": {"next": {"type": "object"}, "value": {"type": "string"}}}}}"#,
    );
}

#[test]
fn a_legacy_definitions_table_at_the_root_expands_too() {
    assert_shaped(
        r##"{"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}},
            "definitions": {"A": {"type": "string"}}}"##,
        r#"{"type": "object", "properties": {"a": {"type": "string"}}}"#,
    );
}

#[test]
fn an_expanded_definition_uses_its_own_nested_table_first() {
    assert_shaped(
        r##"{"type": "object", "properties": {"a": {"$ref": "#/$defs/Outer"}},
            "$defs": {
                "Outer": {"type": "object", "properties": {"inner": {"$ref": "#/$defs/Inner"}},
                           "$defs": {"Inner": {"type": "string"}}},
                "Inner": {"type": "integer"}}}"##,
        r#"{"type": "object", "properties": {"a": {"type": "object", "properties": {"inner": {"type": "string"}}}}}"#,
    );
}

#[test]
fn a_definitions_table_that_is_not_an_object_is_rejected() {
    let error = shape_parameters(
        object(r#"{"type": "object", "$defs": null}"#),
        SchemaFormat::ResponseSchema,
        false,
        &settings(DEFAULT_DEPTH),
    )
    .expect_err("a null $defs cannot be expanded");
    assert!(matches!(error, VertexSchemaError::DefinitionsNotObject));
}

#[test]
fn a_null_branch_leaves_the_other_branches_nullable() {
    assert_shaped(
        r#"{"anyOf": [{"type": "string"}, {"type": "null"}]}"#,
        r#"{"anyOf": [{"type": "string", "nullable": true}]}"#,
    );
}

#[rstest]
#[case::two_leading_nulls(
    r#"{"anyOf": [{"type": "null"}, {"type": "null"}, {"type": "string"}], "title": "T"}"#,
    r#"{"anyOf": [{"type": "null", "nullable": true, "title": "T"},
                  {"type": "string", "nullable": true, "title": "T"}]}"#
)]
#[case::two_trailing_nulls(
    r#"{"anyOf": [{"type": "string"}, {"type": "null"}, {"type": "null"}], "title": "T"}"#,
    r#"{"anyOf": [{"type": "string", "nullable": true, "title": "T"},
                  {"type": "null", "nullable": true, "title": "T"}]}"#
)]
#[case::separated_nulls(
    r#"{"anyOf": [{"type": "null"}, {"type": "string"}, {"type": "null"}]}"#,
    r#"{"anyOf": [{"type": "string", "nullable": true}]}"#
)]
fn adjacent_null_branches_keep_litellms_removal_skip(#[case] text: &str, #[case] expected: &str) {
    assert_shaped(text, expected);
}

#[rstest]
#[case::no_branches(r#"{"anyOf": []}"#)]
#[case::only_null(r#"{"anyOf": [{"type": "null"}]}"#)]
#[case::nested_under_items(r#"{"type": "array", "items": {"anyOf": []}}"#)]
fn an_anyof_without_a_non_null_branch_is_rejected(#[case] text: &str) {
    let error = shape_parameters(
        object(text),
        SchemaFormat::ResponseSchema,
        false,
        &settings(DEFAULT_DEPTH),
    )
    .expect_err("Vertex rejects a schema with no branch left");
    assert!(matches!(error, VertexSchemaError::NullOnlyAnyOf));
}

#[test]
fn an_empty_branch_object_becomes_an_object_type() {
    assert_shaped(
        r#"{"anyOf": [{"type": "string"}, {}]}"#,
        r#"{"anyOf": [{"type": "string"}, {"type": "object"}]}"#,
    );
}

#[rstest]
#[case::primitive_union(
    r#"{"type": ["string", "null"], "default": "x", "title": "T"}"#,
    r#"{"anyOf": [{"type": "string", "title": "T"}, {"type": "null", "title": "T"}]}"#
)]
#[case::object_and_array_branches(
    r#"{"type": ["object", "array"], "properties": {"a": {"type": "string"}},
        "items": {"type": "string"}, "minItems": 1, "required": ["a"], "additionalProperties": false}"#,
    r#"{"anyOf": [
        {"type": "object", "items": {"type": "string"}, "required": ["a"],
         "properties": {"a": {"type": "string"}}, "minItems": 1},
        {"type": "object", "items": {"type": "string"}, "required": ["a"],
         "properties": {"a": {"type": "string"}}, "minItems": 1}]}"#
)]
#[case::array_union(
    r#"{"type": ["array", "null"], "items": {"type": "string"}, "minItems": 1}"#,
    r#"{"anyOf": [{"type": "array", "items": {"type": "string"}, "minItems": 1}, {"type": "null"}]}"#
)]
#[case::primitive_branch_keeps_no_object_fields(
    r#"{"type": ["string", "object"], "properties": {"a": {"type": "string"}}, "required": ["a"]}"#,
    r#"{"anyOf": [{"type": "string"},
                  {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}]}"#
)]
#[case::nested_in_properties(
    r#"{"type": "object", "properties": {"p": {"type": ["string", "null"]}}}"#,
    r#"{"type": "object", "properties": {"p": {"anyOf": [{"type": "string"}, {"type": "null"}]}}}"#
)]
#[case::nested_in_items(
    r#"{"type": "array", "items": {"type": ["string", "null"]}}"#,
    r#"{"type": "array", "items": {"anyOf": [{"type": "string"}, {"type": "null"}]}}"#
)]
#[case::nested_in_anyof(
    r#"{"anyOf": [{"type": ["string", "null"]}]}"#,
    r#"{"anyOf": [{"anyOf": [{"type": "string"}, {"type": "null"}]}]}"#
)]
fn a_type_array_becomes_an_anyof(#[case] text: &str, #[case] expected: &str) {
    assert_shaped(text, expected);
}

#[test]
fn an_anyof_branch_with_properties_recovers_its_type() {
    assert_shaped(
        r#"{"anyOf": [{"properties": {"a": {"type": "string"}}}]}"#,
        r#"{"anyOf": [{"properties": {"a": {"type": "string"}}, "type": "object"}]}"#,
    );
}

#[rstest]
#[case::single_item_unwraps(
    r#"{"type": ["string"], "enum": ["a"]}"#,
    r#"{"type": "string", "enum": ["a"]}"#
)]
#[case::single_item_keeps_its_spelling(
    r#"{"type": ["INTEGER"], "enum": [1]}"#,
    r#"{"type": "INTEGER"}"#
)]
#[case::empty_array_stays(r#"{"type": [], "title": "t"}"#, r#"{"type": [], "title": "t"}"#)]
#[case::string_spelling_stays(
    r#"{"type": "STRING", "enum": ["a"]}"#,
    r#"{"type": "STRING", "enum": ["a"]}"#
)]
fn a_single_type_array_unwraps_and_type_spelling_survives(
    #[case] text: &str,
    #[case] expected: &str,
) {
    assert_shaped(text, expected);
}

#[rstest]
#[case::empty_string(
    r#"{"type": "string", "enum": ["a", "", "b"]}"#,
    r#"{"type": "string", "enum": ["a", null, "b"]}"#
)]
#[case::other_falsy_values_stay(
    r#"{"type": "string", "enum": ["", 0, false, null, "x"]}"#,
    r#"{"type": "string", "enum": [null, 0, false, null, "x"]}"#
)]
fn an_empty_enum_string_becomes_null(#[case] text: &str, #[case] expected: &str) {
    assert_shaped(text, expected);
}

#[rstest]
#[case::integer_type(r#"{"type": "integer", "enum": [1, 2]}"#, r#"{"type": "integer"}"#)]
#[case::uppercase_string_type(
    r#"{"type": "STRING", "enum": ["a"]}"#,
    r#"{"type": "STRING", "enum": ["a"]}"#
)]
#[case::anyof_without_a_string_branch(
    r#"{"anyOf": [{"type": "integer"}, {"type": "number"}], "enum": [1, 2]}"#,
    r#"{"anyOf": [{"type": "integer"}, {"type": "number"}]}"#
)]
fn an_enum_on_a_non_string_type_is_removed(#[case] text: &str, #[case] expected: &str) {
    assert_shaped(text, expected);
}

#[rstest]
#[case::string_branch_keeps_the_enum(
    r#"{"anyOf": [{"type": "string"}, {"type": "integer"}], "enum": ["x"]}"#,
    r#"{"anyOf": [{"type": "string"}, {"type": "integer"}], "enum": ["x"]}"#
)]
#[case::no_string_branch_drops_the_enum(
    r#"{"anyOf": [{"type": "integer"}], "enum": [1]}"#,
    r#"{"anyOf": [{"type": "integer"}]}"#
)]
fn the_enum_type_pass_reads_the_branches(#[case] text: &str, #[case] expected: &str) {
    let mut schema = object(text);
    fix_enum_types(
        &mut schema,
        Recursion {
            max_depth: DEFAULT_DEPTH,
        },
        0,
    )
    .expect("the enum pass runs");
    assert_eq!(schema, object(expected));
}

#[rstest]
#[case::missing_items(
    r#"{"type": "array"}"#,
    r#"{"type": "array", "items": {"type": "object"}}"#
)]
#[case::empty_items(
    r#"{"type": "array", "items": {}}"#,
    r#"{"type": "array", "items": {"type": "object"}}"#
)]
#[case::uppercase_array(
    r#"{"type": "ARRAY"}"#,
    r#"{"type": "ARRAY", "items": {"type": "object"}}"#
)]
#[case::null_items_stay(
    r#"{"type": "array", "items": null}"#,
    r#"{"type": "array", "items": null}"#
)]
#[case::non_array_stays(r#"{"type": "string"}"#, r#"{"type": "string"}"#)]
#[case::nested_arrays(
    r#"{"type": "array", "items": {"type": "array", "items": {"properties": {}}}}"#,
    r#"{"type": "array", "items": {"type": "array", "items": {"type": "object"}}}"#
)]
#[case::branch_list_members(
    r#"{"anyOf": [{"type": "array"}]}"#,
    r#"{"anyOf": [{"type": "array", "items": {"type": "object"}}]}"#
)]
fn array_types_without_items_receive_object_items(#[case] text: &str, #[case] expected: &str) {
    assert_shaped(text, expected);
}

#[rstest]
#[case::empty_properties(r#"{"properties": {}}"#, r#"{"type": "object"}"#)]
#[case::null_required_is_dropped(
    r#"{"properties": {"a": {"type": "string"}}, "required": null}"#,
    r#"{"properties": {"a": {"type": "string"}}, "type": "object"}"#
)]
#[case::empty_required_stays(
    r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": []}"#,
    r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": []}"#
)]
#[case::one_of_is_not_a_type_source(
    r#"{"oneOf": [{"type": "string"}], "title": "x"}"#,
    r#"{"title": "x"}"#
)]
#[case::no_properties_no_type(r#"{"title": "x"}"#, r#"{"title": "x", "type": "object"}"#)]
fn an_object_recovers_its_type_and_empty_property_maps_go(
    #[case] text: &str,
    #[case] expected: &str,
) {
    assert_shaped(text, expected);
}

#[test]
fn only_the_schema_fields_survive_the_filter() {
    assert_shaped(
        r#"{"type": "object", "additionalProperties": false, "$schema": "x",
            "definitions": {"A": {"type": "string"}},
            "properties": {"a": {"type": "string", "format": "email"}},
            "default": {"x": 1}, "example": 5}"#,
        r#"{"type": "object", "properties": {"a": {"type": "string"}},
            "default": {"x": 1}, "example": 5}"#,
    );
}

#[test]
fn only_enum_and_date_time_formats_survive() {
    assert_shaped(
        r#"{"type": "object", "properties": {
            "a": {"type": "string", "format": "enum"},
            "b": {"type": "string", "format": "date-time"},
            "c": {"type": "string", "format": "email"}}}"#,
        r#"{"type": "object", "properties": {
            "a": {"type": "string", "format": "enum"},
            "b": {"type": "string", "format": "date-time"},
            "c": {"type": "string"}}}"#,
    );
}

#[test]
fn anyof_metadata_is_copied_onto_every_branch() {
    assert_shaped(
        r#"{"anyOf": [{"type": "string", "title": "own"}], "title": "outer", "description": "D"}"#,
        r#"{"anyOf": [{"type": "string", "title": "outer", "description": "D"}]}"#,
    );
}

#[test]
fn anyof_metadata_is_not_copied_when_a_branch_is_not_an_object() {
    let mixed = object(r#"{"anyOf": [{"type": "string"}, "x"], "title": "outer"}"#);
    assert_eq!(
        filter_schema_fields(&mixed),
        object(r#"{"anyOf": [{"type": "string"}, "x"]}"#)
    );
    let null_title = object(r#"{"anyOf": [{"type": "string"}], "title": null}"#);
    assert_eq!(
        filter_schema_fields(&null_title),
        object(r#"{"anyOf": [{"type": "string"}]}"#)
    );
}

#[rstest]
#[case::top_level(
    r#"{"type": "object", "properties": {"b": {"type": "string"}, "a": {"type": "integer"}}, "required": ["a", "b"]}"#,
    r#"{"type": "object", "properties": {"b": {"type": "string"}, "a": {"type": "integer"}},
        "required": ["a", "b"], "propertyOrdering": ["b", "a"]}"#
)]
#[case::nested_array_items(
    r#"{"type": "object", "properties": {"list": {"type": "array",
        "items": {"type": "object", "properties": {"z": {"type": "string"}, "y": {"type": "string"}}}}}}"#,
    r#"{"type": "object", "properties": {"list": {"type": "array",
        "items": {"type": "object", "properties": {"z": {"type": "string"}, "y": {"type": "string"}},
                  "propertyOrdering": ["z", "y"]}}}, "propertyOrdering": ["list"]}"#
)]
#[case::caller_order_wins(
    r#"{"propertyOrdering": ["q"], "properties": {"a": {"propertyOrdering": ["z"],
        "properties": {"b": {"type": "string"}}}}}"#,
    r#"{"propertyOrdering": ["q"], "properties": {"a": {"propertyOrdering": ["z"],
        "properties": {"b": {"type": "string"}}, "type": "object"}}, "type": "object"}"#
)]
fn property_ordering_follows_the_declared_property_order(
    #[case] text: &str,
    #[case] expected: &str,
) {
    assert_ordered(text, expected);
}

#[test]
fn without_the_flag_only_a_caller_supplied_property_ordering_survives() {
    assert_shaped(
        r#"{"properties": {"a": {"properties": {"b": {"type": "string"}}}}, "propertyOrdering": ["a"]}"#,
        r#"{"properties": {"a": {"properties": {"b": {"type": "string"}}, "type": "object"}},
            "propertyOrdering": ["a"], "type": "object"}"#,
    );
}

const NESTED_PROPERTIES: &str =
    r#"{"properties": {"a": {"properties": {"b": {"type": "string"}}}}}"#;

type SchemaPass = fn(&mut Object, Recursion, usize) -> Result<(), VertexSchemaError>;

#[rstest]
#[case::null_pass(convert_anyof_null_to_nullable)]
#[case::type_pass(convert_schema_types)]
#[case::empty_enum_pass(fix_enum_empty_strings)]
#[case::enum_type_pass(fix_enum_types)]
#[case::ordering_pass(set_schema_property_ordering)]
fn a_schema_pass_stops_one_level_past_the_limit(#[case] pass: SchemaPass) {
    let mut inside = object(NESTED_PROPERTIES);
    pass(&mut inside, Recursion { max_depth: 2 }, 0).expect("depth two sits on the limit");
    let mut past = object(NESTED_PROPERTIES);
    let error = pass(&mut past, Recursion { max_depth: 1 }, 0)
        .expect_err("depth two is past a limit of one");
    assert!(matches!(
        error,
        VertexSchemaError::DepthLimitExceeded { limit: 1 }
    ));
}

#[test]
fn the_items_pass_counts_the_properties_mapping_as_a_level() {
    let mut inside = object(NESTED_PROPERTIES);
    process_items(&mut inside, Recursion { max_depth: 4 }, 0).expect("depth four is the limit");
    let mut past = object(NESTED_PROPERTIES);
    let error = process_items(&mut past, Recursion { max_depth: 3 }, 0)
        .expect_err("depth four is past the limit");
    assert!(matches!(
        error,
        VertexSchemaError::DepthLimitExceeded { limit: 3 }
    ));
}

#[rstest]
#[case::configured_limit_is_used(2, true)]
#[case::deep_enough(4, false)]
fn the_pipeline_stops_at_the_configured_depth(#[case] max_depth: usize, #[case] fails: bool) {
    let result = shape_parameters(
        object(NESTED_PROPERTIES),
        SchemaFormat::ResponseSchema,
        false,
        &settings(max_depth),
    );
    if fails {
        let error = result.expect_err("the items pass counts the properties map itself");
        assert!(matches!(
            error,
            VertexSchemaError::DepthLimitExceeded { limit: 2 }
        ));
    } else {
        result.expect("the schema is shallow enough");
    }
}
