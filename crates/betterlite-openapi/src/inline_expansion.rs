use thiserror::Error;

use crate::schema_transforms::object_in;
use crate::schema_transforms::tree::{JsonObject, JsonTree};

mod tests;

const MAX_INLINE_DEPTH: u8 = 32;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InlineError {
    #[error("the schema references #/$defs/{name}, which it does not define")]
    MissingDefinition { name: String },
}

// Swagger UI resolves $ref against the whole document, so an inline body cannot keep #/$defs references
pub fn inline_request_body(
    schema: Option<&JsonObject>,
    example: &JsonObject,
) -> Result<JsonObject, InlineError> {
    let mut media = JsonObject::new();
    if let Some(generated) = schema {
        let no_definitions = JsonObject::new();
        let definitions = generated
            .get("$defs")
            .and_then(object_in)
            .unwrap_or(&no_definitions);
        let inlined = inline_object(generated, definitions, 0)?;
        media.insert("schema".to_owned(), inlined);
    }
    media.insert("example".to_owned(), JsonTree::Object(example.clone()));
    let content = JsonObject::from([("application/json".to_owned(), JsonTree::Object(media))]);
    Ok(JsonObject::from([
        ("required".to_owned(), JsonTree::Flag(true)),
        ("content".to_owned(), JsonTree::Object(content)),
    ]))
}

fn inline(tree: &JsonTree, definitions: &JsonObject, depth: u8) -> Result<JsonTree, InlineError> {
    if depth >= MAX_INLINE_DEPTH {
        return Ok(tree.clone());
    }
    match tree {
        JsonTree::Object(object) => inline_object(object, definitions, depth),
        JsonTree::Array(items) => items
            .iter()
            .map(|item| inline(item, definitions, depth.saturating_add(1)))
            .collect::<Result<Vec<_>, _>>()
            .map(JsonTree::Array),
        JsonTree::Null | JsonTree::Flag(_) | JsonTree::Number(_) | JsonTree::Text(_) => {
            Ok(tree.clone())
        }
    }
}

fn inline_object(
    object: &JsonObject,
    definitions: &JsonObject,
    depth: u8,
) -> Result<JsonTree, InlineError> {
    let child_depth = depth.saturating_add(1);
    if let Some(JsonTree::Text(reference)) = object.get("$ref")
        && let Some(name) = reference.strip_prefix("#/$defs/")
    {
        let definition = definitions
            .get(name)
            .ok_or_else(|| InlineError::MissingDefinition {
                name: name.to_owned(),
            })?;
        let target = inline(definition, definitions, child_depth)?;
        let mut merged = object_in(&target).cloned().unwrap_or_default();
        for (key, value) in object {
            if key != "$ref" {
                merged.insert(key.clone(), inline(value, definitions, child_depth)?);
            }
        }
        return Ok(JsonTree::Object(merged));
    }
    // Swagger UI cannot resolve a discriminator mapping once its targets are inlined
    if let Some(property_name) = object.get("propertyName")
        && object.contains_key("mapping")
    {
        let discriminator = JsonObject::from([("propertyName".to_owned(), property_name.clone())]);
        return Ok(JsonTree::Object(discriminator));
    }
    let mut inlined = JsonObject::new();
    for (key, value) in object {
        if key != "$defs" {
            inlined.insert(key.clone(), inline(value, definitions, child_depth)?);
        }
    }
    Ok(JsonTree::Object(inlined))
}
