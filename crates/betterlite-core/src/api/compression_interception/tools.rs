use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub const CONTENT_RETRIEVE_TOOL_NAME: &str = "litellm_content_retrieve";

const TOOL_TYPE_FIELD: &str = "type";
const FUNCTION_FIELD: &str = "function";
const NAME_FIELD: &str = "name";

#[must_use]
pub fn has_retrieval_tool(tools: Option<&Json>) -> bool {
    let Some(Json::List(items)) = tools else {
        return false;
    };
    items.iter().any(is_retrieval_tool)
}

#[must_use]
pub fn merge_tools(existing: &[Json], compressed: &[Json]) -> Vec<Json> {
    let mut merged = existing.to_vec();
    if merged.iter().any(is_retrieval_tool) {
        return merged;
    }
    merged.extend_from_slice(compressed);
    merged
}

fn is_retrieval_tool(tool: &Json) -> bool {
    let Json::Object(fields) = tool else {
        return false;
    };
    let Some(Json::Text(kind)) = fields.get(TOOL_TYPE_FIELD) else {
        return false;
    };
    match kind.as_str() {
        "function" => function_name(fields) == Some(CONTENT_RETRIEVE_TOOL_NAME),
        "custom" => tool_name(fields) == Some(CONTENT_RETRIEVE_TOOL_NAME),
        _ => false,
    }
}

fn function_name(fields: &IndexMap<String, Json>) -> Option<&str> {
    let Some(Json::Object(function)) = fields.get(FUNCTION_FIELD) else {
        return None;
    };
    tool_name(function)
}

fn tool_name(fields: &IndexMap<String, Json>) -> Option<&str> {
    let Some(Json::Text(name)) = fields.get(NAME_FIELD) else {
        return None;
    };
    Some(name.as_str())
}
