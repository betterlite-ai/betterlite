use betterlite_prompts::arguments::json::{Json, Object};

use super::error::VertexSchemaError;
use super::recursion::Recursion;

pub(crate) fn process_items(
    schema: &mut Object,
    recursion: Recursion,
    depth: i64,
) -> Result<(), VertexSchemaError> {
    recursion.check(depth)?;
    let is_array = schema
        .get("type")
        .and_then(Json::as_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("array"));
    if is_array && needs_items(schema) {
        let mut items = Object::new();
        items.insert("type".to_owned(), Json::Text("object".to_owned()));
        schema.insert("items".to_owned(), Json::Object(items));
    }
    let child_depth = depth.saturating_add(1);
    for value in schema.values_mut() {
        match value {
            Json::Object(child) => process_items(child, recursion, child_depth)?,
            Json::List(items) => process_object_list(items, recursion, child_depth)?,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
        }
    }
    Ok(())
}

fn process_object_list(
    items: &mut [Json],
    recursion: Recursion,
    depth: i64,
) -> Result<(), VertexSchemaError> {
    for item in items {
        if let Json::Object(child) = item {
            process_items(child, recursion, depth)?;
        }
    }
    Ok(())
}

// Vertex rejects an array without items, and LiteLLM fills in an empty or missing items with an object
fn needs_items(schema: &Object) -> bool {
    match schema.get("items") {
        None => true,
        Some(Json::Object(items)) => items.is_empty(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => false,
    }
}
