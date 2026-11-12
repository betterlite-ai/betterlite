use betterlite_prompts::arguments::json::{Json, Object};

use super::error::VertexSchemaError;
use super::recursion::Recursion;

pub(crate) fn set_schema_property_ordering(
    schema: &mut Object,
    recursion: Recursion,
    depth: usize,
) -> Result<(), VertexSchemaError> {
    recursion.check(depth)?;
    let names: Option<Vec<String>> = match schema.get("properties") {
        Some(Json::Object(properties)) => Some(properties.keys().cloned().collect()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    if let Some(ordering_names) = names {
        if !schema.contains_key("propertyOrdering") {
            schema.insert(
                "propertyOrdering".to_owned(),
                Json::List(ordering_names.into_iter().map(Json::Text).collect()),
            );
        }
        if let Some(Json::Object(properties)) = schema.get_mut("properties") {
            order_properties(properties, recursion, depth.saturating_add(1))?;
        }
    }
    if let Some(Json::Object(items)) = schema.get_mut("items") {
        set_schema_property_ordering(items, recursion, depth.saturating_add(1))?;
    }
    Ok(())
}

fn order_properties(
    properties: &mut Object,
    recursion: Recursion,
    depth: usize,
) -> Result<(), VertexSchemaError> {
    for property in properties.values_mut() {
        if let Json::Object(child) = property {
            set_schema_property_ordering(child, recursion, depth)?;
        }
    }
    Ok(())
}
