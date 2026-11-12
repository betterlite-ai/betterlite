use betterlite_prompts::arguments::json::{Json, Object};

use super::error::VertexSchemaError;

// Every recursive pass walks the same two child schemas in this order, so one visitor keeps their depths equal
pub(crate) fn visit_properties_and_items<F>(
    schema: &mut Object,
    depth: usize,
    visit: &mut F,
) -> Result<(), VertexSchemaError>
where
    F: FnMut(&mut Object, usize) -> Result<(), VertexSchemaError>,
{
    let child_depth = depth.saturating_add(1);
    if let Some(Json::Object(properties)) = schema.get_mut("properties") {
        for property in properties.values_mut() {
            if let Json::Object(child) = property {
                visit(child, child_depth)?;
            }
        }
    }
    if let Some(Json::Object(items)) = schema.get_mut("items") {
        visit(items, child_depth)?;
    }
    Ok(())
}
