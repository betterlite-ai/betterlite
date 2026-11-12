use betterlite_prompts::arguments::json::{Json, Object};

use super::children::visit_properties_and_items;
use super::error::VertexSchemaError;
use super::recursion::Recursion;

pub(crate) fn fix_enum_types(
    schema: &mut Object,
    recursion: Recursion,
    depth: usize,
) -> Result<(), VertexSchemaError> {
    recursion.check(depth)?;
    if matches!(schema.get("enum"), Some(Json::List(_))) && !enum_survives(schema) {
        schema.shift_remove("enum");
    }
    visit_properties_and_items(schema, depth, &mut |child, child_depth| {
        fix_enum_types(child, recursion, child_depth)
    })?;
    if let Some(Json::List(branches)) = schema.get_mut("anyOf") {
        for branch in branches {
            if let Json::Object(child) = branch {
                fix_enum_types(child, recursion, depth.saturating_add(1))?;
            }
        }
    }
    Ok(())
}

// Vertex only accepts an enum on a string type, here or on one anyOf branch
fn enum_survives(schema: &Object) -> bool {
    if schema
        .get("type")
        .and_then(Json::as_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("string"))
    {
        return true;
    }
    match schema.get("anyOf") {
        Some(Json::List(branches)) => branches.iter().any(|branch| {
            branch
                .as_object()
                .and_then(|object| object.get("type"))
                .and_then(Json::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case("string"))
        }),
        _ => false,
    }
}
