use betterlite_prompts::arguments::json::{Json, Object};

use super::children::visit_properties_and_items;
use super::error::VertexSchemaError;
use super::recursion::Recursion;

pub(crate) fn set_schema_property_ordering(
    schema: &mut Object,
    recursion: Recursion,
    depth: i64,
) -> Result<(), VertexSchemaError> {
    recursion.check(depth)?;
    let names: Option<Vec<String>> = match schema.get("properties") {
        Some(Json::Object(properties)) => Some(properties.keys().cloned().collect()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    if let Some(ordering_names) = names
        && !schema.contains_key("propertyOrdering")
    {
        schema.insert(
            "propertyOrdering".to_owned(),
            Json::List(ordering_names.into_iter().map(Json::Text).collect()),
        );
    }
    visit_properties_and_items(schema, depth, &mut |child, child_depth| {
        set_schema_property_ordering(child, recursion, child_depth)
    })
}
