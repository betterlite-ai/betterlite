use betterlite_prompts::arguments::json::{Json, Object};

use super::error::VertexSchemaError;
use super::recursion::Recursion;

pub(crate) fn convert_anyof_null_to_nullable(
    schema: &mut Object,
    recursion: Recursion,
    depth: usize,
) -> Result<(), VertexSchemaError> {
    recursion.check(depth)?;
    if let Some(Json::List(branches)) = schema.get_mut("anyOf") {
        let contains_null = strip_null_branches(branches)?;
        if contains_null {
            mark_nullable(branches);
        }
    }
    if let Some(Json::Object(properties)) = schema.get_mut("properties") {
        for property in properties.values_mut() {
            if let Json::Object(child) = property {
                convert_anyof_null_to_nullable(child, recursion, depth.saturating_add(1))?;
            }
        }
    }
    if let Some(Json::Object(items)) = schema.get_mut("items") {
        convert_anyof_null_to_nullable(items, recursion, depth.saturating_add(1))?;
    }
    Ok(())
}

// LiteLLM walks the live list while removing the null branch it is on, so two adjacent null branches leave the second one in the anyOf
fn strip_null_branches(branches: &mut Vec<Json>) -> Result<bool, VertexSchemaError> {
    let mut contains_null = false;
    let mut index = 0;
    while index < branches.len() {
        if branches.get(index).is_some_and(is_null_branch) {
            branches.remove(index);
            contains_null = true;
            continue;
        }
        if let Some(Json::Object(branch)) = branches.get_mut(index)
            && branch.is_empty()
        {
            branch.insert("type".to_owned(), Json::Text("object".to_owned()));
        }
        index = index.saturating_add(1);
    }
    if branches.is_empty() {
        return Err(VertexSchemaError::NullOnlyAnyOf);
    }
    Ok(contains_null)
}

fn is_null_branch(branch: &Json) -> bool {
    branch
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(Json::as_str)
        == Some("null")
}

fn mark_nullable(branches: &mut [Json]) {
    for branch in branches {
        if let Json::Object(object) = branch {
            object.insert("nullable".to_owned(), Json::Bool(true));
        }
    }
}
