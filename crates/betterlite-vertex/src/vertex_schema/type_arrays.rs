use betterlite_prompts::arguments::json::{Json, Object};

use super::error::VertexSchemaError;
use super::recursion::Recursion;

const OBJECT_OR_ARRAY: [&str; 2] = ["object", "array"];

const TYPE_SPECIFIC_FIELDS: [&str; 8] = [
    "properties",
    "required",
    "additionalProperties",
    "items",
    "minItems",
    "maxItems",
    "minProperties",
    "maxProperties",
];

pub(crate) fn convert_schema_types(
    schema: &mut Object,
    recursion: Recursion,
    depth: i64,
) -> Result<(), VertexSchemaError> {
    recursion.check(depth)?;
    if let Some(type_value) = schema.get("type").cloned() {
        match type_value {
            Json::List(types) if types.len() > 1 => convert_type_array(schema, &types),
            Json::List(types) => {
                if let Some(single) = types.first() {
                    schema.insert("type".to_owned(), single.clone());
                }
            }
            // LiteLLM reassigns a string type to itself, so the caller's spelling reaches Vertex unchanged
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {}
        }
    }
    recurse_children(schema, recursion, depth)
}

// A multi-type field becomes anyOf, and the object and array branches each take a copy of the constraints beside `type`
fn convert_type_array(schema: &mut Object, types: &[Json]) {
    let mut branches: Vec<Json> = Vec::with_capacity(types.len());
    for type_value in types {
        let Some(name) = type_value.as_str() else {
            continue;
        };
        branches.push(Json::Object(branch_for_type(schema, name)));
    }
    // a primitive union keeps these fields; process_items still walks them and counts the properties mapping as a level
    let moves_type_fields = types
        .iter()
        .filter_map(Json::as_str)
        .any(|name| OBJECT_OR_ARRAY.contains(&name));
    if moves_type_fields {
        for field in TYPE_SPECIFIC_FIELDS {
            schema.shift_remove(field);
        }
    }
    schema.insert("anyOf".to_owned(), Json::List(branches));
    schema.shift_remove("type");
}

fn branch_for_type(schema: &Object, name: &str) -> Object {
    let mut branch = Object::new();
    branch.insert("type".to_owned(), Json::Text(name.to_owned()));
    if OBJECT_OR_ARRAY.contains(&name) {
        for field in TYPE_SPECIFIC_FIELDS {
            if let Some(value) = schema.get(field) {
                branch.insert((*field).to_owned(), value.clone());
            }
        }
    }
    branch
}

fn recurse_children(
    schema: &mut Object,
    recursion: Recursion,
    depth: i64,
) -> Result<(), VertexSchemaError> {
    let child_depth = depth.saturating_add(1);
    if let Some(Json::Object(properties)) = schema.get_mut("properties") {
        for property in properties.values_mut() {
            if let Json::Object(child) = property {
                convert_schema_types(child, recursion, child_depth)?;
            }
        }
    }
    if let Some(Json::Object(items)) = schema.get_mut("items") {
        convert_schema_types(items, recursion, child_depth)?;
    }
    if let Some(Json::List(branches)) = schema.get_mut("anyOf") {
        for branch in branches {
            if let Json::Object(child) = branch {
                convert_schema_types(child, recursion, child_depth)?;
            }
        }
    }
    Ok(())
}
