use std::collections::{BTreeMap, BTreeSet};

use crate::schema_transforms::object_in;
use crate::schema_transforms::tree::{JsonObject, JsonTree};

const DEFS_PREFIX: &str = "#/$defs/";

type Shape<'tree> = (BTreeSet<&'tree str>, BTreeSet<&'tree str>);

// a renamed definition changes the references inside the others, so collisions repeat until none is new
pub(super) fn fixed_renames(
    schemas: &JsonObject,
    definitions: &JsonObject,
    namespace: &str,
) -> BTreeMap<String, String> {
    let mut renames = BTreeMap::new();
    loop {
        let mut next = renames.clone();
        for (name, definition) in definitions {
            let Some(existing) = schemas.get(name) else {
                continue;
            };
            let incoming = rewrite_refs(definition, &renames);
            let existing_shape = shape(existing);
            if *existing != incoming
                && (existing_shape.is_none() || existing_shape != shape(&incoming))
            {
                next.insert(name.clone(), format!("{namespace}_{name}"));
            }
        }
        if next == renames {
            return renames;
        }
        renames = next;
    }
}

// LiteLLM compares property names and required names only, and a schema without properties has no shape
fn shape(tree: &JsonTree) -> Option<Shape<'_>> {
    let object = object_in(tree)?;
    let properties = object.get("properties").and_then(object_in)?;
    if properties.is_empty() {
        return None;
    }
    let mut required = BTreeSet::new();
    if let Some(JsonTree::Array(names)) = object.get("required") {
        for name in names {
            if let JsonTree::Text(text) = name {
                required.insert(text.as_str());
            }
        }
    }
    Some((properties.keys().map(String::as_str).collect(), required))
}

pub(super) fn rewrite_refs(tree: &JsonTree, renames: &BTreeMap<String, String>) -> JsonTree {
    match tree {
        JsonTree::Object(object) => JsonTree::Object(rewrite_object(object, renames)),
        JsonTree::Array(items) => JsonTree::Array(
            items
                .iter()
                .map(|item| rewrite_refs(item, renames))
                .collect(),
        ),
        JsonTree::Null | JsonTree::Flag(_) | JsonTree::Number(_) | JsonTree::Text(_) => {
            tree.clone()
        }
    }
}

pub(super) fn rewrite_object(
    object: &JsonObject,
    renames: &BTreeMap<String, String>,
) -> JsonObject {
    let mut rewritten = JsonObject::new();
    for (key, value) in object.iter().filter(|(key, _)| *key != "$defs") {
        rewritten.insert(key.clone(), rewrite_entry(key, value, renames));
    }
    rewritten
}

fn rewrite_entry(key: &str, value: &JsonTree, renames: &BTreeMap<String, String>) -> JsonTree {
    if key == "$ref"
        && let JsonTree::Text(reference) = value
        && reference.starts_with(DEFS_PREFIX)
    {
        // LiteLLM strips every occurrence of the prefix, not only the leading one
        let name = reference.replace(DEFS_PREFIX, "");
        let component = renames.get(&name).unwrap_or(&name);
        return JsonTree::Text(format!("#/components/schemas/{component}"));
    }
    rewrite_refs(value, renames)
}
