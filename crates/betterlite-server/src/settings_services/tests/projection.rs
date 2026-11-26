use crate::settings_services::error::SettingsSchemaError;
use crate::settings_services::schema::root_schema;

use super::support::{TestRecord, json, parse_json, projection, schema, values};

const OPTIONAL_SCHEMA: &str = r#"{
    "description": "root description",
    "properties": {
        "max_budget": {"description": "outer budget", "anyOf": [{"type": "number"}, {"type": "null"}]},
        "budget_duration": {"description": "outer duration", "anyOf": [{"type": "string"}]},
        "teams": {"description": "outer teams", "anyOf": [{"type": "null"}]},
        "models": {
            "description": "outer models",
            "anyOf": [{"type": "array", "items": {"type": "string"}}, {"type": "null"}]
        },
        "no_description": {"type": "integer"}
    }
}"#;

const OPTIONAL_PROJECTION: &str = r#"{
    "values": {"max_budget": null, "models": ["a"]},
    "field_schema": {
        "description": "root description",
        "properties": {
            "max_budget": {"description": "outer budget", "type": "number"},
            "budget_duration": {"description": "outer duration", "type": "string"},
            "teams": {"description": "outer teams", "type": "string"},
            "models": {"description": "outer models", "type": "array", "items": {"type": "string"}},
            "no_description": {"description": "", "type": "integer"}
        }
    }
}"#;

const REFERENCED_SCHEMA: &str = r##"{
    "properties": {
        "models": {"type": "array", "items": {"$ref": "#/$defs/ModelName"}},
        "missing": {"type": "array", "items": {"$ref": "#/$defs/Missing"}},
        "no_enum": {"type": "array", "items": {"$ref": "#/$defs/NoEnum"}},
        "choice": {"type": "array", "items": {"enum": ["a", "b"], "type": "string"}},
        "plain": {"type": "array", "items": {"type": "integer", "description": "an id"}}
    },
    "$defs": {"ModelName": {"enum": ["gpt-4o"]}, "NoEnum": {"type": "string"}},
    "definitions": {"ModelName": {"enum": ["legacy"]}}
}"##;

const REFERENCED_PROJECTION: &str = r#"{
    "values": {"models": ["gpt-4o"]},
    "field_schema": {
        "description": "",
        "properties": {
            "models": {"description": "", "type": "array", "items": {"enum": ["gpt-4o"]}},
            "missing": {"description": "", "type": "array"},
            "no_enum": {"description": "", "type": "array"},
            "choice": {"description": "", "type": "array", "items": {"enum": ["a", "b"], "type": "string"}},
            "plain": {"description": "", "type": "array", "items": {"type": "integer", "description": "an id"}}
        },
        "ModelName": {"description": "", "properties": {}}
    }
}"#;

const DEFINITIONS_SCHEMA: &str = r##"{
    "description": "root",
    "properties": {
        "models": {"type": "array", "items": {"$ref": "#/$defs/ModelName"}},
        "legacy": {"type": "array", "items": {"$ref": "#/definitions/Legacy"}}
    },
    "$defs": {"ModelName": {"enum": ["gpt-4o"]}},
    "definitions": {
        "Legacy": {
            "description": "legacy section",
            "properties": {
                "model_name": {"description": "the model name"},
                "team_id": {}
            }
        }
    }
}"##;

const DEFINITIONS_PROJECTION: &str = r#"{
    "values": {"models": ["gpt-4o"]},
    "field_schema": {
        "description": "root",
        "properties": {
            "models": {"description": "", "type": "array", "items": {"enum": ["gpt-4o"]}},
            "legacy": {"description": "", "type": "array"}
        },
        "Legacy": {
            "description": "legacy section",
            "properties": {
                "model_name": {"description": "the model name"},
                "team_id": {"description": ""}
            }
        }
    }
}"#;

const LEGACY_SCHEMA: &str = r##"{
    "properties": {"kind": {"type": "array", "items": {"$ref": "#/definitions/Kind"}}},
    "definitions": {"Kind": {"enum": ["a", "b"]}}
}"##;

const LEGACY_PROJECTION: &str = r#"{
    "values": {},
    "field_schema": {
        "description": "",
        "properties": {
            "kind": {"description": "", "type": "array", "items": {"enum": ["a", "b"]}}
        },
        "Kind": {"description": "", "properties": {}}
    }
}"#;

// _resolve_non_null_variant and _schema_property_entry: the first non-null variant types the field, the outer field describes it
#[test]
fn optional_unions_read_the_first_non_null_variant_and_the_outer_description() {
    let record = TestRecord {
        schema: schema(OPTIONAL_SCHEMA),
        values: values(&[("max_budget", json!(null)), ("models", json!(["a"]))]),
    };
    assert_eq!(projection(&record), parse_json(OPTIONAL_PROJECTION));
}

// _schema_items_entry: a $ref contributes only its definition's enum, and a missing definition or enum omits items
#[test]
fn referenced_items_expose_only_the_definition_enum() {
    let record = TestRecord {
        schema: schema(REFERENCED_SCHEMA),
        values: values(&[("models", json!(["gpt-4o"]))]),
    };
    assert_eq!(projection(&record), parse_json(REFERENCED_PROJECTION));
}

// _root_schema: $defs owns lookups when present, while definitions still become nested description sections
#[test]
fn defs_resolve_references_while_definitions_become_nested_sections() {
    let record = TestRecord {
        schema: schema(DEFINITIONS_SCHEMA),
        values: values(&[("models", json!(["gpt-4o"]))]),
    };
    assert_eq!(projection(&record), parse_json(DEFINITIONS_PROJECTION));
}

// with no $defs the legacy definitions answer top-level lookups too, matched by the reference's final path segment
#[test]
fn definitions_resolve_references_when_defs_are_absent() {
    let record = TestRecord {
        schema: schema(LEGACY_SCHEMA),
        values: values(&[]),
    };
    assert_eq!(projection(&record), parse_json(LEGACY_PROJECTION));
}

#[test]
fn schema_without_properties_is_refused() {
    let raw = json!({"type": "object"});
    assert!(matches!(
        root_schema(&raw),
        Err(SettingsSchemaError::MissingProperties)
    ));
}

#[test]
fn definitions_that_are_not_schema_nodes_are_refused() {
    let raw = json!({"properties": {}, "definitions": []});
    assert!(matches!(
        root_schema(&raw),
        Err(SettingsSchemaError::InvalidDefinitions {
            section: "definitions",
            ..
        })
    ));
}
