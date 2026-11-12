use betterlite_prompts::arguments::json::{Json, Object};

use super::children::visit_properties_and_items;
use super::error::VertexSchemaError;
use super::recursion::Recursion;

pub(crate) fn fix_enum_empty_strings(
    schema: &mut Object,
    recursion: Recursion,
    depth: usize,
) -> Result<(), VertexSchemaError> {
    recursion.check(depth)?;
    if let Some(Json::List(values)) = schema.get_mut("enum") {
        for value in values {
            if matches!(value, Json::Text(text) if text.is_empty()) {
                *value = Json::Null;
            }
        }
    }
    visit_properties_and_items(schema, depth, &mut |child, child_depth| {
        fix_enum_empty_strings(child, recursion, child_depth)
    })
}
