use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use thiserror::Error;

pub type JsonObject = Map<String, Value>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("schema root must be an object")]
    RootObject,
    #[error("referenced definition `{0}` is missing")]
    MissingDefinition(String),
    #[error("schema component `{0}` is not an object")]
    InvalidComponent(String),
}

pub fn add_schema_to_components(
    openapi: &mut Value,
    schema_name: &str,
    schema: Value,
) -> Result<(), SchemaError> {
    let root = openapi.as_object_mut().ok_or(SchemaError::RootObject)?;
    let components = object_child(root, "components")?;
    let schemas = object_child(components, "schemas")?;
    let definitions = schema
        .as_object()
        .and_then(|object| object.get("$defs"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let renames = fixed_renames(schemas, &definitions, schema_name);
    for (name, definition) in definitions {
        let target = renames.get(&name).cloned().unwrap_or(name);
        schemas.insert(target, rewrite_refs(definition, &renames));
    }
    schemas.insert(schema_name.to_owned(), rewrite_refs(schema, &renames));
    Ok(())
}

pub fn add_request_body_to_paths(
    openapi: &mut Value,
    paths: &[String],
    schema_ref: &str,
) -> Result<(), SchemaError> {
    let root = openapi.as_object_mut().ok_or(SchemaError::RootObject)?;
    let schemas = root
        .get("components")
        .and_then(Value::as_object)
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .ok_or_else(|| SchemaError::InvalidComponent(schema_ref.to_owned()))?;
    let schema_name = schema_ref.rsplit('/').next().unwrap_or(schema_ref);
    let schema = schemas
        .get(schema_name)
        .and_then(Value::as_object)
        .ok_or_else(|| SchemaError::InvalidComponent(schema_name.to_owned()))?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = schema
        .get("required")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let expanded_properties = properties
        .into_iter()
        .map(|(name, value)| {
            let expanded = expanded_field(&name, value);
            (name, expanded)
        })
        .collect::<Map<_, _>>();
    let expanded = serde_json::json!({
        "type": "object",
        "required": required,
        "properties": expanded_properties,
    });
    let paths_object = object_child(root, "paths")?;
    for path in paths {
        let Some(path_item) = paths_object.get_mut(path).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(post) = path_item.get_mut("post").and_then(Value::as_object_mut) else {
            continue;
        };
        post.insert(
            "requestBody".to_owned(),
            serde_json::json!({
                "required": true,
                "content": {"application/json": {"schema": expanded}},
            }),
        );
        if let Some(parameters) = post.get_mut("parameters").and_then(Value::as_array_mut) {
            parameters.retain(|parameter| {
                parameter.as_object().and_then(|object| object.get("in"))
                    == Some(&Value::String("path".to_owned()))
            });
        }
    }
    Ok(())
}

pub fn add_request_schema(
    openapi: &mut Value,
    schema_name: &str,
    schema: Option<Value>,
    paths: &[String],
    _operation_name: &str,
) -> Result<(), SchemaError> {
    let Some(schema) = schema else {
        return Ok(());
    };
    add_schema_to_components(openapi, schema_name, schema)?;
    let reference = format!("#/components/schemas/{schema_name}");
    add_request_body_to_paths(openapi, paths, &reference)
}

fn object_child<'a>(
    object: &'a mut JsonObject,
    key: &str,
) -> Result<&'a mut JsonObject, SchemaError> {
    let entry = object
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        return Err(SchemaError::InvalidComponent(key.to_owned()));
    }
    match entry {
        Value::Object(value) => Ok(value),
        _ => Err(SchemaError::InvalidComponent(key.to_owned())),
    }
}

fn fixed_renames(
    schemas: &JsonObject,
    definitions: &JsonObject,
    namespace: &str,
) -> BTreeMap<String, String> {
    let mut renames = BTreeMap::new();
    loop {
        let mut changed = false;
        for (name, definition) in definitions {
            let Some(existing) = schemas.get(name) else {
                continue;
            };
            let candidate = rewrite_refs(definition.clone(), &renames);
            if !same_shape(existing, &candidate) {
                let renamed = format!("{namespace}_{name}");
                if renames.get(name) != Some(&renamed) {
                    renames.insert(name.clone(), renamed);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    renames
}

fn same_shape(existing: &Value, incoming: &Value) -> bool {
    if existing == incoming {
        return true;
    }
    let Some(existing) = existing.as_object() else {
        return false;
    };
    let Some(incoming) = incoming.as_object() else {
        return false;
    };
    let Some(existing_props) = existing.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let Some(incoming_props) = incoming.get("properties").and_then(Value::as_object) else {
        return false;
    };
    if existing_props.keys().collect::<BTreeSet<_>>()
        != incoming_props.keys().collect::<BTreeSet<_>>()
    {
        return false;
    }
    required_set(existing.get("required")) == required_set(incoming.get("required"))
}

fn required_set(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn rewrite_refs(value: Value, renames: &BTreeMap<String, String>) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter_map(|(key, value)| {
                    if key == "$defs" {
                        return None;
                    }
                    let value = if key == "$ref" {
                        value
                            .as_str()
                            .and_then(|reference| reference.strip_prefix("#/$defs/"))
                            .map(|name| {
                                Value::String(format!(
                                    "#/components/schemas/{}",
                                    renames.get(name).unwrap_or(&name.to_owned())
                                ))
                            })
                            .unwrap_or_else(|| rewrite_refs(value, renames))
                    } else {
                        rewrite_refs(value, renames)
                    };
                    Some((key, value))
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| rewrite_refs(value, renames))
                .collect(),
        ),
        other => other,
    }
}

fn expanded_field(name: &str, value: Value) -> Value {
    if name == "messages" {
        let mut object = value.as_object().cloned().unwrap_or_default();
        object.insert(
            "example".to_owned(),
            serde_json::json!([{"role": "user", "content": "Hello, how are you?"}]),
        );
        Value::Object(object)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_keeps_existing_shape() {
        let mut document =
            serde_json::json!({"components":{"schemas":{"Block":{"type":"integer"}}}});
        add_schema_to_components(
            &mut document,
            "Request",
            serde_json::json!({"$defs":{"Block":{"type":"integer"}}}),
        )
        .expect("schema should be added");
        assert_eq!(
            document["components"]["schemas"]["Block"]["type"],
            "integer"
        );
        assert!(document["components"]["schemas"]["Request"].is_object());
    }

    #[test]
    fn rewrite_removes_defs_and_changes_refs() {
        let mut document = serde_json::json!({});
        add_schema_to_components(
            &mut document,
            "Request",
            serde_json::json!({"$ref":"#/$defs/Block", "$defs":{"Block":{"type":"string"}}}),
        )
        .expect("schema should be added");
        assert_eq!(
            document["components"]["schemas"]["Request"]["$ref"],
            "#/components/schemas/Block"
        );
        assert!(
            document["components"]["schemas"]["Request"]
                .get("$defs")
                .is_none()
        );
    }

    #[test]
    fn fallback_returns_without_schema() {
        let mut document = serde_json::json!({"paths": {"/chat": {"post": {}}}});
        add_request_schema(
            &mut document,
            "Request",
            None,
            &["/chat".to_owned()],
            "chat",
        )
        .expect("fallback is non-failing");
        assert!(
            document["paths"]["/chat"]["post"]
                .get("requestBody")
                .is_none()
        );
    }
}
