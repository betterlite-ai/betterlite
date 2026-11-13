#![cfg(test)]

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};

use super::response::apply_response_format;

const DEPTH: i64 = 100;

fn settings() -> Settings {
    Settings {
        max_recurse_depth: DEPTH,
    }
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the fixture literal is a JSON object")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn transform(format: &str, model: &str) -> Object {
    let mut optional_params = Object::new();

    apply_response_format(&object(format), model, &mut optional_params, &settings())
        .expect("the fixture schema is supported");

    optional_params
}

const MODERN_FORMAT: &str = r##"{
    "type": "json_schema",
    "json_schema": {
        "name": "Plan",
        "strict": true,
        "schema": {
            "$defs": {
                "Step": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}},
                    "additionalProperties": false
                }
            },
            "type": "object",
            "strict": true,
            "properties": {"step": {"$ref": "#/$defs/Step"}},
            "additionalProperties": false
        }
    }
}"##;

const MODERN_SCHEMA: &str = r##"{
    "$defs": {
        "Step": {
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "additionalProperties": false
        }
    },
    "type": "object",
    "properties": {"step": {"$ref": "#/$defs/Step"}},
    "additionalProperties": false
}"##;

const LEGACY_FORMAT: &str = r##"{
    "type": "json_schema",
    "json_schema": {
        "schema": {
            "$defs": {
                "Step": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}},
                    "additionalProperties": false
                }
            },
            "type": "object",
            "properties": {
                "step": {"$ref": "#/$defs/Step"},
                "note": {"type": ["string", "null"]}
            },
            "additionalProperties": false,
            "strict": true
        }
    }
}"##;

#[test]
fn modern_models_keep_standard_json_schema() {
    let mapped = transform(MODERN_FORMAT, "gemini-2.5-flash");

    assert_eq!(
        mapped.get("response_mime_type"),
        Some(&text("application/json"))
    );
    assert_eq!(
        mapped.get("response_json_schema"),
        Some(&Json::Object(object(MODERN_SCHEMA)))
    );
    assert!(!mapped.contains_key("response_schema"));
}

#[test]
fn older_models_shape_the_openapi_schema() {
    let mapped = transform(LEGACY_FORMAT, "gemini-1.5-pro");

    assert_eq!(
        mapped.get("response_mime_type"),
        Some(&text("application/json"))
    );
    assert!(!mapped.contains_key("response_json_schema"));
    let Some(Json::Object(schema)) = mapped.get("response_schema") else {
        panic!("the legacy path sets response_schema");
    };
    assert_eq!(schema.get("type"), Some(&text("OBJECT")));
    assert_eq!(
        schema.get("propertyOrdering"),
        Some(&Json::List(vec![text("step"), text("note")]))
    );
    assert!(!schema.contains_key("$defs"));
    assert!(!schema.contains_key("strict"));
    assert!(!schema.contains_key("additionalProperties"));
    let Some(Json::Object(step)) = schema
        .get("properties")
        .and_then(Json::as_object)
        .and_then(|properties| properties.get("step"))
    else {
        panic!("the referenced definition is inlined");
    };
    assert_eq!(step.get("type"), Some(&text("OBJECT")));
    assert_eq!(
        step.get("propertyOrdering"),
        Some(&Json::List(vec![text("name")]))
    );
    assert!(!step.contains_key("additionalProperties"));
}

#[test]
fn response_schema_wins_over_the_json_schema_schema() {
    let format = r#"{
        "type": "json_schema",
        "response_schema": {
            "type": "object",
            "properties": {"chosen": {"type": "string"}}
        },
        "json_schema": {
            "schema": {
                "type": "object",
                "properties": {"ignored": {"type": "string"}}
            }
        }
    }"#;

    let mapped = transform(format, "gemini-2.5-flash");

    assert_eq!(
        mapped.get("response_json_schema"),
        Some(&Json::Object(object(
            r#"{"type": "object", "properties": {"chosen": {"type": "string"}}}"#
        )))
    );
}

#[test]
fn an_empty_schema_is_omitted_but_keeps_the_json_mime_type() {
    let mapped = transform(
        r#"{"type": "text", "response_schema": {}}"#,
        "gemini-2.5-flash",
    );

    assert_eq!(
        mapped.get("response_mime_type"),
        Some(&text("application/json"))
    );
    assert_eq!(mapped.len(), 1);
}

#[test]
fn a_schema_of_the_wrong_type_is_ignored() {
    let mapped = transform(
        r#"{"type": "json_schema", "json_schema": {"schema": "not an object"}}"#,
        "gemini-2.5-flash",
    );

    assert_eq!(
        mapped.get("response_mime_type"),
        Some(&text("application/json"))
    );
    assert_eq!(mapped.len(), 1);
}

#[test]
fn a_json_schema_without_a_schema_sets_nothing() {
    let mapped = transform(
        r#"{"type": "json_schema", "json_schema": {"name": "Plan"}}"#,
        "gemini-2.5-flash",
    );

    assert!(mapped.is_empty());
}

#[test]
fn a_null_schema_sets_only_the_json_mime_type() {
    let mapped = transform(
        r#"{"type": "json_schema", "json_schema": {"schema": null}}"#,
        "gemini-2.5-flash",
    );

    assert_eq!(
        mapped.get("response_mime_type"),
        Some(&text("application/json"))
    );
    assert_eq!(mapped.len(), 1);
}

#[test]
fn a_json_object_response_format_sets_the_json_mime_type() {
    let mapped = transform(r#"{"type": "json_object"}"#, "gemini-2.5-flash");

    assert_eq!(
        mapped.get("response_mime_type"),
        Some(&text("application/json"))
    );
    assert_eq!(mapped.len(), 1);
}

#[test]
fn a_text_response_format_sets_the_text_mime_type() {
    let mapped = transform(r#"{"type": "text"}"#, "gemini-2.5-flash");

    assert_eq!(mapped.get("response_mime_type"), Some(&text("text/plain")));
    assert_eq!(mapped.len(), 1);
}

#[test]
fn an_unknown_response_format_sets_nothing() {
    let mapped = transform(r#"{"type": "audio"}"#, "gemini-1.5-pro");

    assert!(mapped.is_empty());
}

#[test]
fn existing_optional_params_survive_the_mapping() {
    let mut optional_params =
        object(r#"{"temperature": 1, "thinkingConfig": {"includeThoughts": true}}"#);

    apply_response_format(
        &object(r#"{"type": "json_object"}"#),
        "gemini-2.5-flash",
        &mut optional_params,
        &settings(),
    )
    .expect("a plain JSON object response format cannot fail");

    assert_eq!(
        optional_params.get("temperature"),
        Some(&Json::Number(serde_json::Number::from(1_i64)))
    );
    assert_eq!(
        optional_params.get("response_mime_type"),
        Some(&text("application/json"))
    );
    assert_eq!(optional_params.len(), 3);
}
