use std::collections::BTreeMap;

use crate::schema_transforms::renames::{fixed_renames, rewrite_object, rewrite_refs};
use crate::schema_transforms::tree::{JsonObject, JsonTree};

mod renames;
mod tests;
pub mod tree;

pub fn add_schema_to_components(document: &mut JsonObject, schema_name: &str, schema: &JsonObject) {
    let JsonTree::Object(components) = document
        .entry("components".to_owned())
        .or_insert_with(|| JsonTree::Object(JsonObject::new()))
    else {
        return;
    };
    let JsonTree::Object(schemas) = components
        .entry("schemas".to_owned())
        .or_insert_with(|| JsonTree::Object(JsonObject::new()))
    else {
        return;
    };
    let no_definitions = JsonObject::new();
    let definitions = schema
        .get("$defs")
        .and_then(object_in)
        .unwrap_or(&no_definitions);
    let renames = fixed_renames(schemas, definitions, schema_name);
    for (name, definition) in definitions {
        let target = renames.get(name);
        if target.is_none() && schemas.contains_key(name) {
            continue;
        }
        let component = target.unwrap_or(name).clone();
        schemas.insert(component, rewrite_refs(definition, &renames));
    }
    schemas.insert(
        schema_name.to_owned(),
        JsonTree::Object(rewrite_object(schema, &renames)),
    );
}

pub fn add_request_body_to_paths(document: &mut JsonObject, paths: &[&str], schema_ref: &str) {
    let schema_name = schema_ref.rsplit('/').next().unwrap_or(schema_ref);
    let request_body = expanded_request_body(document, schema_name);
    let Some(JsonTree::Object(path_items)) = document.get_mut("paths") else {
        return;
    };
    for path in paths {
        let Some(JsonTree::Object(path_item)) = path_items.get_mut(*path) else {
            continue;
        };
        let Some(JsonTree::Object(operation)) = path_item.get_mut("post") else {
            continue;
        };
        operation.insert("requestBody".to_owned(), request_body.clone());
        if let Some(parameters) = operation.get_mut("parameters") {
            let path_parameters = match parameters {
                JsonTree::Array(items) => items
                    .iter()
                    .filter(|item| {
                        object_in(item).and_then(|parameter| parameter.get("in"))
                            == Some(&JsonTree::Text("path".to_owned()))
                    })
                    .cloned()
                    .collect(),
                JsonTree::Null
                | JsonTree::Flag(_)
                | JsonTree::Number(_)
                | JsonTree::Text(_)
                | JsonTree::Object(_) => Vec::new(),
            };
            *parameters = JsonTree::Array(path_parameters);
        }
    }
}

pub fn add_request_schema(
    document: &mut JsonObject,
    schema_name: &str,
    schema: Option<&JsonObject>,
    paths: &[&str],
) {
    if let Some(generated) = schema {
        add_schema_to_components(document, schema_name, generated);
        let schema_ref = format!("#/components/schemas/{schema_name}");
        add_request_body_to_paths(document, paths, &schema_ref);
    }
}

fn expanded_request_body(document: &JsonObject, schema_name: &str) -> JsonTree {
    let schema = document
        .get("components")
        .and_then(object_in)
        .and_then(|components| components.get("schemas").and_then(object_in))
        .and_then(|schemas| schemas.get(schema_name).and_then(object_in));
    let required = schema
        .and_then(|found| found.get("required"))
        .cloned()
        .unwrap_or_else(|| JsonTree::Array(Vec::new()));
    let fields = schema.and_then(|found| found.get("properties").and_then(object_in));
    let expanded_schema = JsonObject::from([
        ("type".to_owned(), JsonTree::Text("object".to_owned())),
        ("required".to_owned(), required),
        (
            "properties".to_owned(),
            JsonTree::Object(expanded_fields(fields)),
        ),
    ]);
    let media = JsonObject::from([("schema".to_owned(), JsonTree::Object(expanded_schema))]);
    let content = JsonObject::from([("application/json".to_owned(), JsonTree::Object(media))]);
    JsonTree::Object(JsonObject::from([
        ("required".to_owned(), JsonTree::Flag(true)),
        ("content".to_owned(), JsonTree::Object(content)),
    ]))
}

// Swagger UI renders one form field per property, so each field is copied in place of the $ref
fn expanded_fields(fields: Option<&JsonObject>) -> JsonObject {
    let mut properties = JsonObject::new();
    let no_renames = BTreeMap::new();
    for (field, definition) in fields.into_iter().flatten() {
        let mut expanded = object_in(definition)
            .map(|field_schema| rewrite_object(field_schema, &no_renames))
            .unwrap_or_default();
        if field == "messages" {
            let greeting = JsonObject::from([
                ("role".to_owned(), JsonTree::Text("user".to_owned())),
                (
                    "content".to_owned(),
                    JsonTree::Text("Hello, how are you?".to_owned()),
                ),
            ]);
            let example = JsonTree::Array(vec![JsonTree::Object(greeting)]);
            expanded.insert("example".to_owned(), example);
        }
        properties.insert(field.clone(), JsonTree::Object(expanded));
    }
    properties
}

pub(crate) const fn object_in(tree: &JsonTree) -> Option<&JsonObject> {
    match tree {
        JsonTree::Object(object) => Some(object),
        JsonTree::Null
        | JsonTree::Flag(_)
        | JsonTree::Number(_)
        | JsonTree::Text(_)
        | JsonTree::Array(_) => None,
    }
}
