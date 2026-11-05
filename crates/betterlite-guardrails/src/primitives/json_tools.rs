use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::json_value::JsonValue;

const MAX_DEPTH: usize = 50;

#[derive(Deserialize)]
struct SchemaNode {
    #[serde(rename = "type", default)]
    kind: String,
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
        if depth > MAX_DEPTH || !has_type(&node.kind, value) {
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

fn has_type(kind: &str, value: &JsonValue) -> bool {
    match kind {
        "object" => matches!(value, JsonValue::Object(_)),
        "array" => matches!(value, JsonValue::Array(_)),
        "string" => matches!(value, JsonValue::String(_)),
        "number" => matches!(value, JsonValue::Number(_)),
        "integer" => {
            matches!(value, JsonValue::Number(number) if number.is_i64() || number.is_u64())
        }
        "boolean" => matches!(value, JsonValue::Bool(_)),
        "null" => matches!(value, JsonValue::Null),
        _ => true,
    }
}
