#![expect(
    clippy::absolute_paths,
    clippy::derive_partial_eq_without_eq,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::match_same_arms,
    clippy::must_use_candidate,
    clippy::or_fun_call,
    clippy::question_mark,
    clippy::shadow_reuse,
    clippy::shadow_unrelated,
    clippy::wildcard_enum_match_arm,
    reason = "the provider boundary transforms an opaque serialized JSON Schema and mirrors LiteLLM's dynamic schema keys"
)]

use serde::Deserialize as _;
use serde_json::{Map, Value};

const CONSTRAINTS: [(&str, &str); 25] = [
    ("minItems", "minimum number of items: {}"),
    ("maxItems", "maximum number of items: {}"),
    ("uniqueItems", "all array items must be unique"),
    ("contains", "array must contain an item matching: {}"),
    ("minContains", "minimum number of matching items: {}"),
    ("maxContains", "maximum number of matching items: {}"),
    ("prefixItems", "leading items must match, in order: {}"),
    ("minimum", "minimum value: {}"),
    ("maximum", "maximum value: {}"),
    ("exclusiveMinimum", "exclusive minimum value: {}"),
    ("exclusiveMaximum", "exclusive maximum value: {}"),
    ("multipleOf", "must be a multiple of {}"),
    ("minLength", "minimum length: {}"),
    ("maxLength", "maximum length: {}"),
    ("minProperties", "minimum number of properties: {}"),
    ("maxProperties", "maximum number of properties: {}"),
    (
        "patternProperties",
        "properties whose names match each pattern must satisfy: {}",
    ),
    ("propertyNames", "property names must satisfy: {}"),
    ("dependentRequired", "dependent required properties: {}"),
    ("dependentSchemas", "dependent schemas: {}"),
    (
        "unevaluatedProperties",
        "unevaluated properties must satisfy: {}",
    ),
    ("if", "conditional (if): {}"),
    ("then", "conditional (then): {}"),
    ("else", "conditional (else): {}"),
    ("not", "must not match: {}"),
];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OutputFormat {
    #[serde(rename = "type")]
    pub type_name: String,
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseFormat {
    Text,
    JsonSchema { schema: Value },
    JsonObject { schema: Option<Value> },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Tool {
    pub name: String,
    pub input_schema: Value,
}

pub fn filter_anthropic_output_schema(schema: Value) -> Value {
    let Value::Object(schema) = schema else {
        return schema;
    };
    let drops_type = enum_type_conflict(&schema);
    let note = constraint_note(&schema);
    let existing_description = schema
        .get("description")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let mut result = Map::new();
    for (key, value) in schema {
        if CONSTRAINTS.iter().any(|(name, _)| *name == key) || (key == "type" && drops_type) {
            continue;
        }
        if key == "description" && note.is_some() {
            continue;
        }
        let value = match key.as_str() {
            "properties" | "$defs" => filter_named(value, true),
            "items" => filter_named(value, false),
            "anyOf" | "allOf" | "oneOf" => filter_list(value, &key, &mut result),
            _ => value,
        };
        if !(key == "oneOf" && value.is_null()) {
            result.insert(key, value);
        }
    }
    if let Some(note) = note {
        result.remove("description");
        let description = existing_description;
        result.insert(
            "description".to_owned(),
            Value::String(description.map_or(note.clone(), |value| format!("{value} {note}"))),
        );
    }
    if result.get("type").and_then(Value::as_str) == Some("object") {
        result.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
    Value::Object(result)
}

fn filter_named(value: Value, fields: bool) -> Value {
    match value {
        Value::Object(values) if fields => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, filter_anthropic_output_schema(value)))
                .collect(),
        ),
        Value::Object(value) => filter_anthropic_output_schema(Value::Object(value)),
        _ => value,
    }
}

fn filter_list(value: Value, key: &str, result: &mut Map<String, Value>) -> Value {
    let Value::Array(values) = value else {
        return value;
    };
    let filtered = values
        .into_iter()
        .map(filter_anthropic_output_schema)
        .collect::<Vec<_>>();
    if key == "oneOf" {
        let mut any = result
            .remove("anyOf")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        any.extend(filtered);
        result.insert("anyOf".to_owned(), Value::Array(any));
        Value::Null
    } else if key == "anyOf" {
        let mut any = result
            .remove("anyOf")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        any.extend(filtered);
        Value::Array(any)
    } else {
        Value::Array(filtered)
    }
}

fn constraint_note(schema: &Map<String, Value>) -> Option<String> {
    let notes = CONSTRAINTS
        .iter()
        .filter_map(|(key, label)| {
            let value = schema.get(*key)?;
            if value == &Value::Bool(false) {
                return None;
            }
            let rendered = if value.is_object() || value.is_array() {
                python_json(value)
            } else {
                format_value(value)
            };
            Some(label.replace("{}", &rendered))
        })
        .collect::<Vec<_>>();
    (!notes.is_empty()).then(|| format!("Note: {}.", notes.join(", ")))
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => {
            if *value {
                "True".to_owned()
            } else {
                "False".to_owned()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn python_json(value: &Value) -> String {
    match value {
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_json)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    serde_json::to_string(key).unwrap_or_default(),
                    python_json(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn enum_type_conflict(schema: &Map<String, Value>) -> bool {
    let Some(Value::Array(values)) = schema.get("enum") else {
        return false;
    };
    let Some(Value::String(kind)) = schema.get("type") else {
        return false;
    };
    if kind == "array" || kind == "object" {
        return true;
    }
    values.iter().any(|value| !matches_type(value, kind))
}

fn matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number() && !value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

pub fn map_output(value: Option<ResponseFormat>) -> Option<OutputFormat> {
    let Some(value) = value else {
        return None;
    };
    match value {
        ResponseFormat::Text => None,
        ResponseFormat::JsonSchema { schema }
        | ResponseFormat::JsonObject {
            schema: Some(schema),
        } => Some(OutputFormat {
            type_name: "json_schema".to_owned(),
            schema: filter_anthropic_output_schema(expand_definitions(schema)),
        }),
        ResponseFormat::JsonObject { schema: None } => None,
    }
}

fn expand_definitions(schema: Value) -> Value {
    let Value::Object(mut schema) = schema else {
        return schema;
    };
    let definitions = schema
        .remove("$defs")
        .or_else(|| schema.remove("definitions"));
    let Some(Value::Object(definitions)) = definitions else {
        return Value::Object(schema);
    };
    if definitions.is_empty() {
        return Value::Object(schema);
    }
    let schema =
        betterlite_prompts::arguments::json::Object::deserialize(Value::Object(schema)).ok();
    let defs =
        betterlite_prompts::arguments::json::Object::deserialize(Value::Object(definitions)).ok();
    match (schema, defs) {
        (Some(schema), Some(defs)) => {
            betterlite_prompts::definitions::expand::unpack_defs(schema, &defs, Some(10_000_000))
                .ok()
                .and_then(|value| serde_json::to_value(value).ok())
                .unwrap_or(Value::Object(Map::new()))
        }
        _ => Value::Object(Map::new()),
    }
}

pub fn map_tool(value: Option<ResponseFormat>) -> Option<Tool> {
    let Some(value) = value else {
        return None;
    };
    let schema = match value {
        ResponseFormat::Text => return None,
        ResponseFormat::JsonSchema { schema }
        | ResponseFormat::JsonObject {
            schema: Some(schema),
        } => Some(schema),
        ResponseFormat::JsonObject { schema: None } => None,
    };
    Some(json_tool(schema))
}

pub fn create_json_tool_call(schema: Option<Value>) -> Tool {
    let tool = json_tool(schema);
    tool
}

fn json_tool(schema: Option<Value>) -> Tool {
    let input_schema = schema.map_or_else(
        || serde_json::json!({"type":"object", "additionalProperties":true, "properties":{}}),
        |schema| {
            let mut base = serde_json::json!({"type":"object"});
            if let (Value::Object(base), Value::Object(schema)) = (&mut base, schema) {
                base.extend(schema);
            }
            base
        },
    );
    Tool {
        name: "json_tool_call".to_owned(),
        input_schema,
    }
}

mod tests;
