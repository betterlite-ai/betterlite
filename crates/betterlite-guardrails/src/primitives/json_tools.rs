use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::json_value::JsonValue;

const MAX_DEPTH: usize = 50;

#[derive(Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum SchemaType {
    Array,
    Boolean,
    Integer,
    Null,
    Number,
    Object,
    String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TypeSpec {
    One(SchemaType),
    Many(Vec<SchemaType>),
}

#[derive(Deserialize)]
struct SchemaNode {
    #[serde(rename = "type", default)]
    kind: Option<TypeSpec>,
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    properties: IndexMap<String, Self>,
}

pub fn json_parse(text: &str) -> JsonValue {
    match serde_json::from_str(text) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::debug!(%error, "custom code json_parse error");
            JsonValue::Null
        }
    }
}

pub fn json_stringify<Encodable>(value: &Encodable) -> String
where
    Encodable: Serialize,
{
    match serde_json::to_string(value) {
        Ok(encoded) => encoded,
        Err(error) => {
            tracing::warn!(%error, "custom code json_stringify error");
            String::new()
        }
    }
}

pub fn json_schema_valid(object: &JsonValue, schema: &JsonValue) -> bool {
    let parsed =
        serde_json::to_string(schema).and_then(|text| serde_json::from_str::<SchemaNode>(&text));
    match parsed {
        Ok(root) => conforms(object, &root),
        Err(error) => {
            tracing::warn!(%error, "custom code json_schema_valid error");
            false
        }
    }
}

fn conforms(object: &JsonValue, root: &SchemaNode) -> bool {
    let mut pending = vec![(object, root, 0_usize)];
    while let Some((value, node, depth)) = pending.pop() {
        if depth > MAX_DEPTH || !has_type(node.kind.as_ref(), value) {
            return false;
        }
        let JsonValue::Object(fields) = value else {
            continue;
        };
        if node.required.iter().any(|name| !fields.contains_key(name)) {
            return false;
        }
        for (name, child_schema) in &node.properties {
            if let Some(child) = fields.get(name) {
                pending.push((child, child_schema, depth.saturating_add(1)));
            }
        }
    }
    true
}

fn has_type(kind: Option<&TypeSpec>, value: &JsonValue) -> bool {
    let Some(spec) = kind else {
        return true;
    };
    match spec {
        TypeSpec::One(name) => type_matches(name, value),
        TypeSpec::Many(names) => {
            let unique = names
                .iter()
                .enumerate()
                .all(|(index, name)| !names.iter().take(index).any(|other| other == name));
            unique && names.iter().any(|name| type_matches(name, value))
        }
    }
}

fn type_matches(kind: &SchemaType, value: &JsonValue) -> bool {
    match (kind, value) {
        (SchemaType::Object, JsonValue::Object(_))
        | (SchemaType::Array, JsonValue::Array(_))
        | (SchemaType::String, JsonValue::String(_))
        | (SchemaType::Boolean, JsonValue::Bool(_))
        | (SchemaType::Null, JsonValue::Null)
        | (SchemaType::Number, JsonValue::Number(_)) => true,
        (SchemaType::Integer, JsonValue::Number(number)) => number
            .as_f64()
            .is_some_and(|float| float.fract() == 0.0_f64),
        _ => false,
    }
}
